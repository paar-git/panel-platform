/**
 * What a project's state means, and how to say it.
 *
 * The core stores statuses as `RUNNING`, `BUILD_FAILED` and so on. Deciding
 * once, here, which of those is worth alarming about is what keeps the
 * dashboard, the project list and the detail page from disagreeing about
 * whether a project is fine.
 */
import type { ProjectSummary } from '../api';
import type { Tone } from '../ui/primitives';

export interface StatusLook {
  label: string;
  tone: Tone;
  /** True while the core is mid-transition, so controls disable. */
  transitioning: boolean;
}

/**
 * Stopped is neutral, not red.
 *
 * Stopping a project is a normal thing to do on purpose; colouring it as a
 * fault teaches people to ignore red.
 */
export function statusLook(status: string): StatusLook {
  switch (status.toUpperCase()) {
    case 'RUNNING':
      return { label: 'Running', tone: 'ok', transitioning: false };
    case 'STARTING':
      return { label: 'Starting', tone: 'accent', transitioning: true };
    case 'STOPPING':
      return { label: 'Stopping', tone: 'accent', transitioning: true };
    case 'RESTARTING':
      return { label: 'Restarting', tone: 'accent', transitioning: true };
    case 'BUILDING':
      return { label: 'Building', tone: 'accent', transitioning: true };
    case 'DELETING':
      return { label: 'Deleting', tone: 'warn', transitioning: true };
    case 'STOPPED':
      return { label: 'Stopped', tone: 'neutral', transitioning: false };
    case 'CREATED':
      return { label: 'Created', tone: 'neutral', transitioning: false };
    case 'FAILED':
      return { label: 'Failed', tone: 'danger', transitioning: false };
    case 'BUILD_FAILED':
      return { label: 'Build failed', tone: 'danger', transitioning: false };
    case 'CRASHED':
      return { label: 'Crashed', tone: 'danger', transitioning: false };
    default:
      return { label: sentenceCase(status), tone: 'neutral', transitioning: false };
  }
}

/** The health column, which is separate from the status. */
export function healthLook(health: string): StatusLook {
  switch (health.toUpperCase()) {
    case 'HEALTHY':
      return { label: 'Healthy', tone: 'ok', transitioning: false };
    case 'UNHEALTHY':
      return { label: 'Unhealthy', tone: 'danger', transitioning: false };
    case 'STARTING':
      return { label: 'Checking', tone: 'accent', transitioning: true };
    case 'NONE':
    case 'UNKNOWN':
      return { label: 'No health check', tone: 'neutral', transitioning: false };
    default:
      return { label: sentenceCase(health), tone: 'neutral', transitioning: false };
  }
}

export function isRunning(status: string): boolean {
  return status.toUpperCase() === 'RUNNING';
}

export function isFailed(status: string): boolean {
  return ['FAILED', 'BUILD_FAILED', 'CRASHED'].includes(status.toUpperCase());
}

/**
 * A project the user should look at.
 *
 * Two cases: it failed, or it is supposed to be running and is not. The second
 * is the one that matters — a project whose desired state is RUNNING but whose
 * status is STOPPED has stopped without being asked to.
 */
export function needsAttention(project: ProjectSummary): boolean {
  if (isFailed(project.status)) return true;
  const wantsToRun = project.desiredState.toUpperCase() === 'RUNNING';
  const look = statusLook(project.status);
  return wantsToRun && !isRunning(project.status) && !look.transitioning;
}

/** Why it needs attention, in words. */
export function attentionReason(project: ProjectSummary): string {
  if (isFailed(project.status)) return statusLook(project.status).label;
  return 'Should be running';
}

export interface StatusCounts {
  total: number;
  running: number;
  stopped: number;
  failed: number;
  attention: number;
}

export function countByStatus(projects: ProjectSummary[]): StatusCounts {
  return {
    total: projects.length,
    running: projects.filter((project) => isRunning(project.status)).length,
    failed: projects.filter((project) => isFailed(project.status)).length,
    // Anything not running and not failed: stopped, created, or mid-transition.
    stopped: projects.filter((project) => !isRunning(project.status) && !isFailed(project.status))
      .length,
    attention: projects.filter(needsAttention).length,
  };
}

/**
 * The audit log's `project.start` becomes "Started project".
 *
 * The log is written for machines to filter and people to read, and the raw
 * dotted form is the wrong half of that in a feed.
 */
export function describeAction(action: string, targetLabel: string | null): string {
  const known: Record<string, string> = {
    'project.create': 'Created project',
    'project.start': 'Started project',
    'project.stop': 'Stopped project',
    'project.restart': 'Restarted project',
    'project.kill': 'Killed project',
    'project.delete': 'Deleted project',
    'project.update': 'Updated project',
    'file.write': 'Saved file',
    'file.create': 'Created file',
    'file.delete': 'Deleted file',
    'file.rename': 'Renamed file',
    'file.upload': 'Uploaded file',
    'file.import': 'Imported files',
    'app.start': 'Application started',
    'app.shutdown': 'Application stopped',
  };

  const phrase = known[action] ?? sentenceCase(action.replace(/[._]/g, ' '));
  return targetLabel ? `${phrase} ${targetLabel}` : phrase;
}

export function actionTone(result: string): Tone {
  switch (result.toUpperCase()) {
    case 'SUCCESS':
    case 'OK':
      return 'ok';
    case 'FAILURE':
    case 'FAILED':
    case 'ERROR':
      return 'danger';
    case 'DENIED':
      return 'warn';
    default:
      return 'neutral';
  }
}

function sentenceCase(value: string): string {
  const lower = value.toLowerCase().replace(/_/g, ' ');
  return lower.charAt(0).toUpperCase() + lower.slice(1);
}

/**
 * Whether a project's controls should be usable, and why not when they are not.
 *
 * Nothing outside the project blocks it any more. A missing daemon used to,
 * and a missing *runtime* deliberately does not: that is caught when Start is
 * pressed, and answered with the runtime's name and an offer to install it,
 * which is a better answer than a disabled button explaining nothing.
 *
 * Lives here rather than in the view because it is the same decision in two
 * places, and because it is worth testing without rendering anything.
 */
export function runControls(
  project: ProjectSummary,
  options: { busy: boolean },
): { blocked: boolean; reason?: string } {
  const look = statusLook(project.status);

  if (look.transitioning) {
    return { blocked: true, reason: `The project is ${look.label.toLowerCase()}` };
  }
  if (options.busy) {
    return { blocked: true, reason: 'Another action is running' };
  }
  return { blocked: false };
}

// ------------------------------------------------------------ the Run control

/**
 * What the one prominent button should do right now.
 *
 * There is exactly one of these decisions in the product, because the top bar,
 * the project card and the overview all show the same control and any
 * disagreement between them is a bug the user experiences as the app lying.
 *
 * The button is never green once stopping is the primary action: a green
 * control that stops something is how people stop a project they meant to
 * restart.
 */
export interface RunAction {
  /** What pressing it does. `null` while nothing should happen. */
  action: 'start' | 'stop' | null;
  label: string;
  /** `play`, `stop`, or `spinner` while the core is mid-transition. */
  icon: 'play' | 'stop' | 'spinner' | 'warn';
  tone: 'ok' | 'neutral' | 'danger' | 'accent';
  /** True while a transition is in flight, so the control cannot be pressed twice. */
  pending: boolean;
}

export function primaryRunAction(status: string): RunAction {
  switch (status.toUpperCase()) {
    case 'RUNNING':
      return { action: 'stop', label: 'Stop', icon: 'stop', tone: 'neutral', pending: false };
    case 'STARTING':
      return { action: null, label: 'Starting', icon: 'spinner', tone: 'accent', pending: true };
    case 'STOPPING':
      return { action: null, label: 'Stopping', icon: 'spinner', tone: 'accent', pending: true };
    case 'RESTARTING':
      return { action: null, label: 'Restarting', icon: 'spinner', tone: 'accent', pending: true };
    case 'BUILDING':
      return { action: null, label: 'Building', icon: 'spinner', tone: 'accent', pending: true };
    case 'DELETING':
      return { action: null, label: 'Deleting', icon: 'spinner', tone: 'accent', pending: true };
    // A failed project's primary action is still Run — the user's next move is
    // to try again, having read the console. The warning colour is what marks
    // it as different from an ordinary stopped project.
    case 'FAILED':
    case 'BUILD_FAILED':
    case 'CRASHED':
      return { action: 'start', label: 'Run', icon: 'warn', tone: 'danger', pending: false };
    default:
      return { action: 'start', label: 'Run', icon: 'play', tone: 'ok', pending: false };
  }
}
