/**
 * The 22px bar along the bottom.
 *
 * Only facts. There is no git branch here because the core does not read one,
 * and no "Live Share" or "Remote" because there is nothing behind them. What is
 * shown is what this window can actually determine: the project's real state,
 * Monaco's real marker counts, and the real position of the caret in the real
 * open file.
 *
 * The bar takes the accent colour when a project is running and a muted one
 * when it is not, which is the fastest possible answer to "is it up?".
 */
import Icon from './Icon';
import type { CursorPosition } from './CodeEditor';

export default function StatusBar({
  projectStatus,
  running,
  errors,
  warnings,
  language,
  cursor,
  lineEnding,
  dirty,
  saving,
  onProblems,
  onRun,
}: {
  projectStatus: string;
  running: boolean;
  errors: number;
  warnings: number;
  /** Null with no file open. */
  language: string | null;
  cursor: CursorPosition | null;
  lineEnding: 'LF' | 'CRLF' | null;
  dirty: boolean;
  saving: boolean;
  onProblems: () => void;
  onRun: () => void;
}) {
  return (
    <footer
      className={`flex h-[22px] shrink-0 items-center gap-0.5 px-1 text-[12px] text-white ${
        running ? 'bg-vs-status' : 'bg-vs-status-idle'
      }`}
    >
      <StatusItem onClick={onRun} title="Open the Terminal panel to start or stop this project">
        <Icon name={running ? 'stop' : 'play'} size={13} />
        {projectStatus.toLowerCase()}
      </StatusItem>

      <StatusItem onClick={onProblems} title="Problems reported by the editor (Ctrl+Shift+M)">
        <Icon name="error" size={13} />
        {errors}
        <Icon name="warning" size={13} />
        {warnings}
      </StatusItem>

      <span className="flex-1" />

      {saving && <StatusText>Saving…</StatusText>}
      {!saving && dirty && <StatusText>Unsaved</StatusText>}

      {cursor && (
        <StatusText title="Line and column of the caret">
          Ln {cursor.line}, Col {cursor.column}
          {cursor.selected > 0 ? ` (${cursor.selected} selected)` : ''}
        </StatusText>
      )}
      {/* The core reads and writes text as UTF-8 and nothing else, so this is
          a fact rather than a picker pretending to offer a choice. */}
      {language && <StatusText title="The core reads and writes files as UTF-8">UTF-8</StatusText>}
      {lineEnding && <StatusText title="The line endings in this file">{lineEnding}</StatusText>}
      {/* Not a picker: the core decides the language from the file's extension
          and the editor follows it, so there is no choice to offer here. */}
      {language && (
        <StatusText title="The language Monaco is highlighting this file as">{language}</StatusText>
      )}
    </footer>
  );
}

function StatusItem({
  children,
  title,
  onClick,
}: {
  children: React.ReactNode;
  title: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      className="flex h-[22px] items-center gap-1 px-1.5 hover:bg-white/15"
    >
      {children}
    </button>
  );
}

function StatusText({ children, title }: { children: React.ReactNode; title?: string }) {
  return (
    <span title={title} className="flex h-[22px] items-center gap-1 px-1.5">
      {children}
    </span>
  );
}
