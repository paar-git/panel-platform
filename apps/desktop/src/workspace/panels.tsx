/**
 * The four things the bottom panel can show.
 *
 * They are small enough to share a file, and grouping them makes the one rule
 * they have in common obvious: none of them invents anything. Problems come
 * from Monaco, Output is a transcript of operations that ran, Terminal drives
 * the real project process, and Logs shows the core's real logging
 * configuration. Where a capability does not exist — an interactive shell into
 * a project, a streamed container log — the panel says so plainly instead of
 * printing convincing lines.
 */
import Icon from './Icon';
import type { AppSettings, ProjectSummary } from '../api';
import { useTypewriter } from '../lib/typewriter';
import { formatTime, type OutputChannel, type OutputLine } from './output';
import type { Problem } from './problems';

/** Shared by all four: the same centred, quiet message. */
function PanelNotice({ children }: { children: React.ReactNode }) {
  return <p className="px-4 py-3 text-[13px] text-vs-dim">{children}</p>;
}

// ------------------------------------------------------------------ problems

export function ProblemsPanel({
  problems,
  hasOpenFiles,
  onSelect,
}: {
  problems: Problem[];
  hasOpenFiles: boolean;
  onSelect: (problem: Problem) => void;
}) {
  if (problems.length === 0) {
    return (
      <PanelNotice>
        {hasOpenFiles
          ? 'No problems have been detected in the open files.'
          : 'No problems have been detected in the workspace so far.'}
      </PanelNotice>
    );
  }

  return (
    <ul className="py-1">
      {problems.map((problem, index) => (
        <li key={`${problem.path}:${problem.line}:${problem.column}:${index}`}>
          <button
            type="button"
            onClick={() => onSelect(problem)}
            className="flex w-full items-start gap-2 px-3 py-0.5 text-left hover:bg-white/5"
          >
            <span
              className={`mt-0.5 shrink-0 ${
                problem.severity === 'error'
                  ? 'text-red-400'
                  : problem.severity === 'warning'
                    ? 'text-amber-400'
                    : 'text-sky-400'
              }`}
            >
              <Icon name={problem.severity === 'info' ? 'info' : problem.severity} size={14} />
            </span>
            <span className="min-w-0 flex-1 text-[13px] text-vs-text">
              {problem.message}
              <span className="ml-2 text-vs-dim">
                {problem.path} [{problem.line}, {problem.column}]
                {problem.source ? ` · ${problem.source}` : ''}
              </span>
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}

// -------------------------------------------------------------------- output

const CHANNELS: (OutputChannel | 'all')[] = ['all', 'Files', 'Transfers', 'Project', 'Workspace'];

export function OutputPanel({
  lines,
  channel,
  onSelectChannel,
  onClear,
}: {
  lines: OutputLine[];
  channel: OutputChannel | 'all';
  onSelectChannel: (channel: OutputChannel | 'all') => void;
  onClear: () => void;
}) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-2 px-3 py-1.5">
        <label className="text-[11px] text-vs-dim" htmlFor="output-channel">
          Channel
        </label>
        <select
          id="output-channel"
          value={channel}
          onChange={(event) => onSelectChannel(event.target.value as OutputChannel | 'all')}
          className="h-[22px] border border-vs-border bg-vs-editor px-1.5 text-[12px] text-vs-text outline-none"
        >
          {CHANNELS.map((entry) => (
            <option key={entry} value={entry}>
              {entry === 'all' ? 'All channels' : entry}
            </option>
          ))}
        </select>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onClear}
          disabled={lines.length === 0}
          title="Clear Output"
          aria-label="Clear Output"
          className="grid h-5 w-5 place-items-center rounded-[3px] text-vs-text hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent"
        >
          <Icon name="trash" size={14} />
        </button>
      </div>

      {lines.length === 0 ? (
        <PanelNotice>Nothing has been written to this channel yet.</PanelNotice>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto px-3 pb-2 font-mono text-[12px] leading-[1.5] select-text">
          {lines.map((line) => (
            <p key={line.id} className="whitespace-pre-wrap">
              <span className="text-vs-dim">[{formatTime(line.at)}] </span>
              <span className="text-vs-dim">{line.channel} </span>
              <span
                className={
                  line.level === 'error'
                    ? 'text-red-400'
                    : line.level === 'warn'
                      ? 'text-amber-400'
                      : 'text-vs-text'
                }
              >
                {line.text}
              </span>
            </p>
          ))}
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ terminal

export type RunAction = 'start' | 'restart' | 'stop' | 'kill';

/**
 * The project's process.
 *
 * This is where the old console screen went. It is called Terminal because that
 * is where a person looks for "start this thing", but it is honest about what
 * it is: there is no interactive shell into a project and no streamed log, so
 * it offers the controls that exist and says what the state actually is.
 */
/**
 * One line of the transcript, typed out as it arrives.
 *
 * Only ever animates on first mount, which is exactly the behaviour wanted:
 * lines are appended, so a new line mounts and types itself while everything
 * above it — already mounted — stays put. Re-opening the panel does not replay
 * the history, and no "which line is newest" bookkeeping is needed to get that.
 */
function TypedLine({ line }: { line: OutputLine }) {
  const { shown, done } = useTypewriter(line.text);

  return (
    <p className="whitespace-pre-wrap">
      <span className="text-vs-dim">[{formatTime(line.at)}] </span>
      <span className={line.level === 'error' ? 'text-red-400' : 'text-vs-text'}>{shown}</span>
      {!done && (
        <span aria-hidden className="animate-caret ml-px inline-block w-[7px] text-vs-text">
          ▍
        </span>
      )}
    </p>
  );
}

export function TerminalPanel({
  project,
  busy,
  onAction,
  lines,
}: {
  project: ProjectSummary;
  busy: RunAction | null;
  onAction: (action: RunAction) => void;
  /** The Project channel of the transcript: what these buttons actually did. */
  lines: OutputLine[];
}) {
  const running = project.status === 'RUNNING';
  const transitioning = ['STARTING', 'STOPPING', 'RESTARTING'].includes(project.status);
  const disabled = busy !== null || transitioning;

  function reason(action: string): string {
    if (transitioning) return 'The project is already changing state';
    return action;
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-1.5 px-3 py-1.5">
        <RunButton
          icon="play"
          label={busy === 'start' ? 'Starting…' : 'Start'}
          title={reason('Start this project')}
          tone="text-emerald-400"
          disabled={disabled || running}
          onClick={() => onAction('start')}
        />
        <RunButton
          icon="restart"
          label={busy === 'restart' ? 'Restarting…' : 'Restart'}
          title={reason('Restart this project')}
          tone="text-sky-400"
          disabled={disabled || !running}
          onClick={() => onAction('restart')}
        />
        <RunButton
          icon="stop"
          label={busy === 'stop' ? 'Stopping…' : 'Stop'}
          title={reason('Stop this project gracefully')}
          tone="text-vs-text"
          disabled={disabled || !running}
          onClick={() => onAction('stop')}
        />
        <RunButton
          icon="power"
          label={busy === 'kill' ? 'Killing…' : 'Kill'}
          title={reason('Kill this project immediately')}
          tone="text-red-400"
          disabled={disabled || !running}
          onClick={() => onAction('kill')}
        />

        <span className="ml-2 text-[12px] text-vs-dim">
          {project.slug} · {project.status.toLowerCase()} · wants{' '}
          {project.desiredState.toLowerCase()}
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-auto border-t border-vs-border px-3 py-2 font-mono text-[12px] leading-[1.5] select-text">
        {lines.length === 0 ? (
          <p className="text-vs-dim">
            No process action has run in this session. Use the controls above to start, stop or
            restart the project.
          </p>
        ) : (
          lines.map((line) => <TypedLine key={line.id} line={line} />)
        )}
      </div>

      <p className="shrink-0 border-t border-vs-border px-3 py-1.5 text-[11px] text-vs-dim">
        An interactive shell into a running project, and streaming its log, are not built into the
        core yet — so this panel shows what these controls did rather than output nobody collected.
      </p>
    </div>
  );
}

function RunButton({
  icon,
  label,
  title,
  tone,
  disabled,
  onClick,
}: {
  icon: Parameters<typeof Icon>[0]['name'];
  label: string;
  title: string;
  tone: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      className={`flex items-center gap-1.5 rounded-[3px] border border-vs-border px-2 py-0.5 text-[12px] hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent ${tone}`}
    >
      <Icon name={icon} size={13} />
      {label}
    </button>
  );
}

// ---------------------------------------------------------------------- logs

/**
 * Where the core writes its log, and how.
 *
 * Read from the settings the core reports, so it is what is actually in force
 * rather than what a default says it should be.
 */
export function LogsPanel({
  settings,
  onCopyPath,
}: {
  settings: AppSettings | null;
  onCopyPath: (path: string) => void;
}) {
  if (!settings) return <PanelNotice>Reading the core&apos;s settings…</PanelNotice>;

  const rows: { label: string; value: string; path?: boolean }[] = [
    { label: 'Log directory', value: settings.logsDir, path: true },
    { label: 'Level', value: settings.logLevel },
    { label: 'Format', value: settings.logJson ? 'JSON' : 'text' },
    { label: 'Retention', value: `${settings.logRetentionDays} days` },
    { label: 'Data directory', value: settings.dataDir, path: true },
    { label: 'Projects directory', value: settings.projectsDir, path: true },
  ];

  return (
    <div className="px-3 py-2">
      <table className="text-[12px]">
        <tbody>
          {rows.map((row) => (
            <tr key={row.label}>
              <th scope="row" className="py-0.5 pr-4 text-left font-normal text-vs-dim">
                {row.label}
              </th>
              <td className="py-0.5 font-mono text-vs-text select-text">
                {row.value}
                {row.path && (
                  <button
                    type="button"
                    onClick={() => onCopyPath(row.value)}
                    title="Copy this path"
                    aria-label={`Copy the ${row.label.toLowerCase()}`}
                    className="ml-2 align-middle text-vs-dim hover:text-vs-text"
                  >
                    <Icon name="copy" size={13} />
                  </button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="mt-3 text-[11px] text-vs-dim">
        The core writes its log to files in that directory. Reading them back into this panel is not
        built yet, so nothing is shown here that has not been read.
      </p>
    </div>
  );
}
