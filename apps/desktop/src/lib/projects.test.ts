import { describe, expect, it } from 'vitest';

import type { ProjectSummary } from '../api';
import {
  actionTone,
  attentionReason,
  countByStatus,
  describeAction,
  healthLook,
  isFailed,
  needsAttention,
  primaryRunAction,
  runControls,
  statusLook,
} from './projects';

function project(status: string, desiredState = 'RUNNING'): ProjectSummary {
  return {
    id: `id-${status}-${desiredState}`,
    slug: 'demo',
    displayName: 'Demo',
    description: '',
    projectType: 'NODEJS',
    status,
    desiredState,
    color: null,
  };
}

describe('reading a status', () => {
  it('colours a running project green and a failed one red', () => {
    expect(statusLook('RUNNING').tone).toBe('ok');
    expect(statusLook('BUILD_FAILED').tone).toBe('danger');
  });

  it('leaves a stopped project neutral', () => {
    // Stopping a project on purpose is normal. Colouring it as a fault is how
    // people learn to ignore red.
    expect(statusLook('STOPPED').tone).toBe('neutral');
  });

  it('marks the in-between states as transitioning, so controls disable', () => {
    for (const status of ['STARTING', 'STOPPING', 'RESTARTING', 'BUILDING']) {
      expect(statusLook(status).transitioning).toBe(true);
    }
    expect(statusLook('RUNNING').transitioning).toBe(false);
  });

  it('renders a status it has never seen rather than showing nothing', () => {
    expect(statusLook('SOME_NEW_STATE').label).toBe('Some new state');
  });

  it('does not care about case', () => {
    expect(statusLook('running').label).toBe('Running');
  });
});

describe('reading health', () => {
  it('separates healthy from having no check at all', () => {
    expect(healthLook('HEALTHY').tone).toBe('ok');
    expect(healthLook('UNHEALTHY').tone).toBe('danger');
    expect(healthLook('NONE').label).toBe('No health check');
  });
});

describe('which projects need attention', () => {
  it('flags anything that failed', () => {
    expect(needsAttention(project('BUILD_FAILED'))).toBe(true);
    expect(isFailed('CRASHED')).toBe(true);
  });

  it('flags a project that should be running and is not', () => {
    expect(needsAttention(project('STOPPED', 'RUNNING'))).toBe(true);
    expect(attentionReason(project('STOPPED', 'RUNNING'))).toBe('Should be running');
  });

  it('leaves a project alone when it was asked to stop', () => {
    expect(needsAttention(project('STOPPED', 'STOPPED'))).toBe(false);
  });

  it('does not flag one that is still on its way up', () => {
    // A starting project is not a problem; it is a project starting.
    expect(needsAttention(project('STARTING', 'RUNNING'))).toBe(false);
  });

  it('leaves a healthy running project alone', () => {
    expect(needsAttention(project('RUNNING', 'RUNNING'))).toBe(false);
  });
});

describe('counting', () => {
  it('splits the list by state', () => {
    const counts = countByStatus([
      project('RUNNING', 'RUNNING'),
      project('RUNNING', 'RUNNING'),
      project('STOPPED', 'STOPPED'),
      project('FAILED', 'RUNNING'),
    ]);

    expect(counts).toEqual({ total: 4, running: 2, stopped: 1, failed: 1, attention: 1 });
  });

  it('counts an empty list without dividing by anything', () => {
    expect(countByStatus([])).toEqual({
      total: 0,
      running: 0,
      stopped: 0,
      failed: 0,
      attention: 0,
    });
  });
});

describe('describing an audit entry', () => {
  it('turns a dotted action into a phrase', () => {
    expect(describeAction('project.start', null)).toBe('Started project');
  });

  it('appends the target when the log recorded one', () => {
    expect(describeAction('project.start', 'demo-api')).toBe('Started project demo-api');
  });

  it('renders an action it does not know rather than the raw string', () => {
    expect(describeAction('backup.prune', null)).toBe('Backup prune');
  });

  it('colours the result', () => {
    expect(actionTone('SUCCESS')).toBe('ok');
    expect(actionTone('FAILURE')).toBe('danger');
    expect(actionTone('DENIED')).toBe('warn');
    expect(actionTone('whatever')).toBe('neutral');
  });
});

describe('runControls', () => {
  it('leaves a stopped project usable', () => {
    expect(runControls(project('STOPPED'), { busy: false })).toEqual({ blocked: false });
  });

  // Nothing outside the project blocks it any more. A missing daemon used to;
  // a missing runtime deliberately does not, because that is caught when Start
  // is pressed and answered with an offer to install it.
  it('never blocks for a reason outside the project', () => {
    expect(runControls(project('STOPPED'), { busy: false }).reason).toBeUndefined();
  });

  it('blocks while the project is transitioning', () => {
    const { blocked, reason } = runControls(project('STARTING'), { busy: false });
    expect(blocked).toBe(true);
    expect(reason).toBe('The project is starting');
  });

  it('blocks while another action is running', () => {
    expect(runControls(project('STOPPED'), { busy: true }).blocked).toBe(true);
  });
});

describe('primaryRunAction', () => {
  it('offers Run when the project is down', () => {
    const action = primaryRunAction('STOPPED');
    expect(action.action).toBe('start');
    expect(action.label).toBe('Run');
    expect(action.tone).toBe('ok');
  });

  /** A green button that stops things is how people stop the wrong project. */
  it('is never green once stopping is the primary action', () => {
    const action = primaryRunAction('RUNNING');
    expect(action.action).toBe('stop');
    expect(action.tone).not.toBe('ok');
  });

  it('does nothing at all while a transition is in flight', () => {
    for (const status of ['STARTING', 'STOPPING', 'RESTARTING', 'BUILDING']) {
      const action = primaryRunAction(status);
      expect(action.action, status).toBeNull();
      expect(action.pending, status).toBe(true);
      expect(action.icon, status).toBe('spinner');
    }
  });

  /** After a crash the next move is to try again, not to be blocked. */
  it('still offers Run after a failure, marked as a failure', () => {
    for (const status of ['FAILED', 'CRASHED', 'BUILD_FAILED']) {
      const action = primaryRunAction(status);
      expect(action.action, status).toBe('start');
      expect(action.tone, status).toBe('danger');
    }
  });

  it('treats a status it has never heard of as runnable rather than stuck', () => {
    expect(primaryRunAction('SOMETHING_NEW').action).toBe('start');
  });
});
