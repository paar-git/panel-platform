/**
 * The update manager.
 *
 * One window for the whole update, opened from the top bar, the editor's Help
 * menu, the command palette and Settings — because the previous arrangement had
 * three places each rendering their own idea of the same install, and they drift
 * apart the moment one of them is edited.
 *
 * # Presentation only
 *
 * Everything this file decides is a layout decision. *What* state the update is
 * in comes from `screenFor`, what it says comes from `describeScreen`, and which
 * controls exist comes from `controlsFor` — all pure functions in `update.ts`
 * with tests. There is deliberately no `switch (screen.state)` here beyond
 * choosing which block of markup to lay out, so the screen cannot disagree with
 * the model about what is happening.
 *
 * # What is real, and what is deliberately absent
 *
 * Every figure is derived from bytes the backend actually reported. Percentage,
 * size, rate and time remaining each render nothing rather than a guess when the
 * data cannot support one, and no stage advances on a timer: a stage the updater
 * never reports is a stage this window never shows.
 *
 * There is **no pause and no cancel**. Tauri's updater downloads, verifies and
 * installs in one uninterruptible call. Offering either control would mean
 * replacing it with a downloader of our own, which would also mean
 * re-implementing signature verification against the key compiled into the
 * build — a weaker security path bought for a button. Minimise is offered
 * instead: the window gets out of the way and the update keeps going.
 */
import { useEffect, useRef, useState } from 'react';

import { minimizeWindow, restartApp } from '../api';
import {
  controlsFor,
  describeScreen,
  formatBytes,
  formatRate,
  formatRemaining,
  isWorking,
  rateBetween,
  screenCaption,
  screenFor,
  screenPercent,
  secondsRemaining,
  smoothRate,
  type ScreenTone,
  type UpdateScreen,
} from '../update';
import { updateStore, useUpdate } from '../useUpdate';
import Icon, { type IconName } from '../ui/Icon';
import Logo from '../ui/Logo';
import { Button } from '../ui/primitives';
import { toast } from '../ui/toast';

/** The install stages, in the order the updater reports them. */
const STAGES: { id: UpdateScreen['state']; label: string; icon: IconName }[] = [
  { id: 'downloading', label: 'Download', icon: 'download' },
  { id: 'verifying', label: 'Verify', icon: 'shield' },
  { id: 'installing', label: 'Install', icon: 'container' },
  { id: 'restart_required', label: 'Restart', icon: 'restart' },
];

/**
 * Which stage the rail is on.
 *
 * `preparing` is index 0 rather than -1: the download stage has been entered,
 * it just has nothing to report yet. `completed` is past the end, which fills
 * the whole rail.
 */
function stageIndex(screen: UpdateScreen): number {
  if (screen.state === 'preparing') return 0;
  if (screen.state === 'completed') return STAGES.length;
  return STAGES.findIndex((stage) => stage.id === screen.state);
}

/**
 * Track how fast the download is going.
 *
 * Kept in the view rather than the store because it is a property of *watching*
 * a download, not of the download: a window opened halfway through should build
 * its own estimate rather than inherit one made from bytes it never saw.
 */
export function useTransferRate(screen: UpdateScreen): number | null {
  const [rate, setRate] = useState<number | null>(null);
  const previous = useRef<{ at: number; bytes: number } | null>(null);

  const bytes = screen.state === 'downloading' ? screen.downloadedBytes : null;

  useEffect(() => {
    if (bytes === null) {
      previous.current = null;
      setRate(null);
      return;
    }

    const sample = { at: performance.now(), bytes };
    const last = previous.current;
    previous.current = sample;
    if (!last) return;

    const measured = rateBetween(last, sample);
    if (measured !== null) setRate((current) => smoothRate(current, measured));
  }, [bytes]);

  return rate;
}

/** Tone to the tokens that carry it. No literal colours: themes must follow. */
const TONES: Record<ScreenTone, { mark: string; ink: string }> = {
  neutral: { mark: 'bg-raised text-muted', ink: 'text-ink' },
  accent: { mark: 'bg-accent-soft text-accent', ink: 'text-ink' },
  ok: { mark: 'bg-ok-soft text-ok', ink: 'text-ok' },
  danger: { mark: 'bg-danger-soft text-danger', ink: 'text-danger' },
};

function StageRail({ screen }: { screen: UpdateScreen }) {
  const active = stageIndex(screen);

  return (
    <ol className="flex items-center gap-1.5" aria-label="Update stages">
      {STAGES.map((stage, index) => {
        const done = index < active;
        const current = index === active;
        return (
          <li key={stage.id} className="flex min-w-0 flex-1 items-center gap-1.5">
            <span
              aria-current={current ? 'step' : undefined}
              className={`grid h-5 w-5 shrink-0 place-items-center rounded-full ${
                done
                  ? 'bg-accent text-canvas'
                  : current
                    ? 'bg-accent-soft text-accent'
                    : 'bg-raised text-faint'
              }`}
            >
              <Icon
                name={done ? 'check' : stage.icon}
                size={11}
                className={current ? 'animate-mark' : undefined}
              />
            </span>
            <span
              className={`truncate text-[11px] ${
                current ? 'text-ink' : done ? 'text-muted' : 'text-faint'
              }`}
            >
              {stage.label}
            </span>
            {index < STAGES.length - 1 && (
              <span
                aria-hidden
                className={`h-px min-w-2 flex-1 ${done ? 'bg-accent' : 'bg-edge'}`}
              />
            )}
          </li>
        );
      })}
    </ol>
  );
}

/** The callbacks the window needs. Passed in so the view can be tested alone. */
export interface UpdateManagerActions {
  onInstall: () => void;
  onCheck: () => void;
  onMinimize: () => void;
  onRestart: () => void;
  onClose: () => void;
}

/**
 * The window itself, as a pure function of the screen.
 *
 * Exported separately from the container so every one of the twelve states can
 * be rendered in a test by passing the state, with no store, no network and no
 * download.
 */
export function UpdateManagerView({
  screen,
  currentVersion,
  actions,
}: {
  screen: UpdateScreen;
  currentVersion: string;
  actions: UpdateManagerActions;
}) {
  const copy = describeScreen(screen);
  const controls = controlsFor(screen);
  const tone = TONES[copy.tone];
  const rate = useTransferRate(screen);

  const percent = screenPercent(screen);
  const caption = screenCaption(screen);
  const working = isWorking(screen);
  const showRail = [
    'preparing',
    'downloading',
    'verifying',
    'installing',
    'restart_required',
    'completed',
  ].includes(screen.state);

  const speed = screen.state === 'downloading' ? formatRate(rate) : null;
  const remaining =
    screen.state === 'downloading'
      ? formatRemaining(secondsRemaining(screen.downloadedBytes, screen.totalBytes, rate))
      : null;

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/70 p-4 sm:p-8"
      // No click-outside close: it would be a way to abandon an install by
      // missing a button. Closing goes through the control, which knows when it
      // is safe.
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Software update"
        aria-busy={working}
        className="animate-pop flex max-h-full w-full max-w-[520px] flex-col overflow-hidden rounded-[14px] border border-edge bg-surface shadow-[0_24px_64px_rgba(0,0,0,0.6)]"
      >
        {/* --------------------------------------------------------- header */}
        <div className="flex shrink-0 items-center gap-3 border-b border-edge px-5 py-4">
          <Logo size={36} title="Panel Platform" />
          <div className="min-w-0 flex-1">
            <p className="truncate text-[14px] font-semibold text-ink">Panel Platform</p>
            <p className="truncate text-[12px] text-muted">Software update</p>
          </div>
          <button
            type="button"
            aria-label="Close"
            title={
              controls.closeEnabled
                ? 'Close'
                : 'The update is running. This closes when it is safe to.'
            }
            disabled={!controls.closeEnabled}
            onClick={actions.onClose}
            className="shrink-0 rounded-[8px] p-1 text-muted hover:bg-raised hover:text-ink disabled:pointer-events-none disabled:opacity-30"
          >
            <Icon name="close" size={16} />
          </button>
        </div>

        {/* ----------------------------------------------------------- body */}
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
          <div className="flex items-start gap-3">
            <span
              className={`grid h-10 w-10 shrink-0 place-items-center rounded-[12px] ${tone.mark}`}
            >
              <Icon
                name={copy.icon}
                size={18}
                className={screen.state === 'checking' ? 'animate-spin-slow' : undefined}
              />
            </span>
            <div className="min-w-0 flex-1">
              <h2 className={`text-[15px] font-semibold ${tone.ink}`}>{copy.title}</h2>
              <p className="mt-1 text-[13px] leading-relaxed text-muted">{copy.detail}</p>
            </div>
          </div>

          {/* Release notes. Only where there are some — an empty panel headed
              "What's new" says less than no panel. */}
          {screen.state === 'available' && screen.notes.trim() !== '' && (
            <div className="mt-4 rounded-[10px] border border-edge bg-canvas p-3">
              <p className="text-[11px] font-medium tracking-wide text-faint uppercase">
                What&apos;s new
              </p>
              <p className="mt-1.5 text-[13px] leading-relaxed whitespace-pre-line text-ink">
                {screen.notes.trim()}
              </p>
              {screen.publishedAt && (
                <p className="mt-2 text-[12px] text-faint">Published {screen.publishedAt}</p>
              )}
            </div>
          )}

          {showRail && (
            <div className="mt-5">
              <StageRail screen={screen} />
            </div>
          )}

          {/* The bar. Indeterminate when there is no honest percentage, rather
              than parked at 0% while bytes are plainly arriving. */}
          {(working || screen.state === 'restart_required' || screen.state === 'completed') && (
            <div className="mt-3">
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-raised">
                <div
                  role="progressbar"
                  aria-label="Update progress"
                  aria-valuenow={percent ?? undefined}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  className={`h-full rounded-full bg-accent ${
                    percent === null ? 'w-1/3 animate-pulse' : 'transition-[width] duration-200'
                  }`}
                  style={percent === null ? undefined : { width: `${percent}%` }}
                />
              </div>

              {/* Wraps rather than truncates: on a narrow window the rate and
                  the time left drop to a second line instead of vanishing. */}
              <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-muted">
                {caption && <span className="text-ink">{caption}</span>}
                {percent !== null && screen.state === 'downloading' && (
                  <span className="tabular">{percent}%</span>
                )}
                {speed && <span className="tabular">{speed}</span>}
                {remaining && <span className="tabular">{remaining}</span>}
                {screen.state === 'downloading' && screen.totalBytes === null && (
                  <span className="tabular">{formatBytes(screen.downloadedBytes)} so far</span>
                )}
              </div>
            </div>
          )}

          <div className="mt-5 border-t border-edge pt-3 text-[12px] text-faint">
            Installed version {currentVersion}
          </div>
        </div>

        {/* -------------------------------------------------------- controls */}
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-edge bg-canvas px-5 py-3.5">
          {controls.minimize && (
            <Button variant="ghost" icon="sidebar" onClick={actions.onMinimize}>
              Minimise
            </Button>
          )}
          {controls.check && (
            <Button icon="refresh" onClick={actions.onCheck}>
              Check again
            </Button>
          )}
          {controls.retryInstall && (
            <Button variant="primary" icon="refresh" onClick={actions.onInstall}>
              Try again
            </Button>
          )}
          {controls.install && (
            <Button variant="primary" icon="download" onClick={actions.onInstall}>
              Install now
            </Button>
          )}
          {controls.restart && (
            <Button variant="primary" icon="restart" onClick={actions.onRestart}>
              Restart now
            </Button>
          )}
          {/* Held open during the wait so the row never empties: a footer with
              one disabled control still reads as a window that is doing
              something, where an empty one reads as a window that has hung. */}
          {working && !controls.restart && (
            <Button disabled pending>
              Working…
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * The connected window.
 *
 * The container is the only part that knows about the store, the Tauri commands
 * or the toast host, which is what keeps the view above testable by passing it
 * a value.
 */
export default function UpdateManager({
  open,
  currentVersion,
  onClose,
}: {
  open: boolean;
  currentVersion: string;
  onClose: () => void;
}) {
  const state = useUpdate();
  const screen = screenFor(state);

  if (!open) return null;

  return (
    <UpdateManagerView
      screen={screen}
      currentVersion={currentVersion}
      actions={{
        onInstall: () => void updateStore.install(),
        onCheck: () => void updateStore.check(),
        onMinimize: () => {
          minimizeWindow().catch(() =>
            toast.error('Could not minimise', 'The window manager refused the request.'),
          );
        },
        onRestart: () => {
          // No success path to report: a restart that works replaces this
          // process, so only the failure is ever seen.
          restartApp().catch(() =>
            toast.error('Could not restart', 'Close and reopen Panel Platform to finish.'),
          );
        },
        onClose,
      }}
    />
  );
}
