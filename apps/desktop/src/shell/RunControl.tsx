/**
 * The one control the user must never have to hunt for.
 *
 * It is a single button whose meaning changes with the project's state, rather
 * than a row of Start/Stop/Restart with two of them always disabled. What it
 * does is decided by `primaryRunAction`, which the project card and the
 * overview read too — so the three can never disagree about whether a project
 * is running.
 *
 * The attached arrow opens the run command, which is a real property of the
 * project rather than a menu of invented scripts.
 */
import { useEffect, useRef, useState } from 'react';

import { errorMessage, type ProjectSummary } from '../api';
import { primaryRunAction, runControls } from '../lib/projects';
import Icon from '../ui/Icon';

export default function RunControl({
  project,
  busy,
  onStart,
  onStop,
  onRestart,
  onOpenConsole,
  onConfigure,
}: {
  project: ProjectSummary | null;
  /** True while an action this control started is still in flight. */
  busy: boolean;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onOpenConsole: () => void;
  onConfigure: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const wrap = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!menuOpen) return undefined;
    function onDown(event: MouseEvent) {
      if (wrap.current !== null && !wrap.current.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === 'Escape') setMenuOpen(false);
    }
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [menuOpen]);

  if (project === null) return null;

  const action = primaryRunAction(project.status);
  const gate = runControls(project, { busy });
  const failed = action.tone === 'danger';

  // Disabled while the core is mid-transition or an action is in flight, which
  // is what stops a double press from queueing a second start behind the first.
  const disabled = action.pending || busy || (action.action === 'start' && gate.blocked);

  const tone =
    action.action === 'stop'
      ? 'border-edge-strong bg-raised text-ink hover:bg-overlay'
      : failed
        ? 'border-danger/40 bg-danger-soft text-danger hover:brightness-125'
        : 'border-ok/40 bg-ok-soft text-ok hover:brightness-125';

  return (
    <div ref={wrap} className="relative flex items-center">
      <button
        type="button"
        disabled={disabled}
        title={
          disabled && gate.reason !== undefined
            ? gate.reason
            : failed
              ? 'The last run failed. Running again will start it fresh.'
              : `${action.label} ${project.displayName}`
        }
        onClick={() => {
          if (action.action === 'start') onStart();
          else if (action.action === 'stop') onStop();
        }}
        className={`flex h-[26px] items-center gap-1.5 rounded-l-[5px] border py-0 pr-2.5 pl-2 text-[12.5px] font-medium transition-colors duration-100 disabled:cursor-not-allowed disabled:opacity-55 ${tone}`}
      >
        {action.icon === 'spinner' ? (
          <span
            aria-hidden
            className="h-3 w-3 animate-spin rounded-full border-[1.5px] border-current border-t-transparent"
          />
        ) : (
          <Icon name={action.icon === 'warn' ? 'alert' : action.icon} size={13} />
        )}
        {action.label}
      </button>

      <button
        type="button"
        aria-label="Run options"
        aria-expanded={menuOpen}
        title="Run options"
        onClick={() => setMenuOpen((open) => !open)}
        className={`flex h-[26px] w-5 items-center justify-center rounded-r-[5px] border border-l-0 transition-colors duration-100 ${tone}`}
      >
        <Icon name="chevron-down" size={11} />
      </button>

      {menuOpen && (
        <div
          role="menu"
          className="absolute top-full right-0 z-50 mt-1 min-w-[190px] rounded-[7px] border border-edge-strong bg-overlay p-1 shadow-lg"
        >
          <MenuItem
            label="Restart"
            disabled={project.status !== 'RUNNING' || busy}
            onClick={() => {
              setMenuOpen(false);
              onRestart();
            }}
          />
          <MenuItem
            label="Open console"
            onClick={() => {
              setMenuOpen(false);
              onOpenConsole();
            }}
          />
          <div className="my-1 h-px bg-edge" aria-hidden />
          {/* Named "configure" rather than listing npm/python commands: the run
              command is a property of the project, and offering a menu of
              scripts this application has not read would be inventing them. */}
          <MenuItem
            label="Configure run command"
            onClick={() => {
              setMenuOpen(false);
              onConfigure();
            }}
          />
        </div>
      )}
    </div>
  );
}

function MenuItem({
  label,
  disabled = false,
  onClick,
}: {
  label: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      onClick={onClick}
      className="flex h-[30px] w-full items-center rounded-[5px] px-2 text-left text-[12.5px] text-ink hover:bg-raised disabled:cursor-not-allowed disabled:text-faint disabled:hover:bg-transparent"
    >
      {label}
    </button>
  );
}

/** Re-exported so callers can report a failure without importing the api module. */
export { errorMessage };
