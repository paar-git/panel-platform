import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  launchStatus: vi.fn(),
  ready: null as (() => void) | null,
  failed: null as ((message: string) => void) | null,
}));

vi.mock('../api', () => ({
  launchStatus: backend.launchStatus,
  onCoreReady: (handler: () => void) => {
    backend.ready = handler;
    return Promise.resolve(() => {
      backend.ready = null;
    });
  },
  onCoreFailed: (handler: (message: string) => void) => {
    backend.failed = handler;
    return Promise.resolve(() => {
      backend.failed = null;
    });
  },
}));

import CoreGate, { holdLaunchStatus } from './CoreGate';

describe('holdLaunchStatus', () => {
  it('does not let a late starting status overwrite ready', () => {
    expect(holdLaunchStatus({ state: 'ready' }, { state: 'starting' })).toEqual({
      state: 'ready',
    });
  });

  it('does not let a late starting status overwrite a failure', () => {
    const failed = { state: 'failed' as const, message: 'no' };
    expect(holdLaunchStatus(failed, { state: 'starting' })).toEqual(failed);
  });
});

describe('CoreGate', () => {
  beforeEach(() => {
    backend.launchStatus.mockReset();
    backend.ready = null;
    backend.failed = null;
  });

  it('keeps children off screen until the core is ready', async () => {
    backend.launchStatus.mockResolvedValue({ state: 'starting' });
    render(
      <CoreGate>
        <p>the shell</p>
      </CoreGate>,
    );
    expect(screen.getByRole('status')).toHaveTextContent('Starting');
    expect(screen.queryByText('the shell')).not.toBeInTheDocument();

    await waitFor(() => {
      expect(backend.launchStatus).toHaveBeenCalled();
    });
    expect(screen.queryByText('the shell')).not.toBeInTheDocument();
  });

  it('shows children once the status command says ready', async () => {
    backend.launchStatus.mockResolvedValue({ state: 'ready' });
    render(
      <CoreGate>
        <p>the shell</p>
      </CoreGate>,
    );
    expect(await screen.findByText('the shell')).toBeInTheDocument();
  });

  it('shows the failure instead of the shell when the core cannot start', async () => {
    backend.launchStatus.mockResolvedValue({
      state: 'failed',
      message: 'the database could not be opened',
    });
    render(
      <CoreGate>
        <p>the shell</p>
      </CoreGate>,
    );
    expect(await screen.findByRole('status')).toHaveTextContent('the database could not be opened');
    expect(screen.queryByText('the shell')).not.toBeInTheDocument();
  });

  it('shows children when the ready event arrives first', async () => {
    backend.launchStatus.mockReturnValue(new Promise(() => undefined));
    render(
      <CoreGate>
        <p>the shell</p>
      </CoreGate>,
    );
    await waitFor(() => {
      expect(backend.ready).not.toBeNull();
    });
    backend.ready?.();
    expect(await screen.findByText('the shell')).toBeInTheDocument();
  });
});
