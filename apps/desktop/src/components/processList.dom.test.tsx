/**
 * The process list, rendered.
 *
 * What matters here is not the layout but two claims the whole multi-process
 * feature rests on: that a failed process is identified *by name* with its own
 * reason attached, and that its restart button restarts that process rather
 * than the project. Both are assertions about wiring, so both are made against
 * the real component with a spy in place of the command.
 */
import { render, screen as dom } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type { ProcessSummary } from '../api';
import { ProcessList } from './ProcessList';

function process(overrides: Partial<ProcessSummary> & { name: string }): ProcessSummary {
  return {
    id: `prc_${overrides.name}`,
    startOrder: 0,
    command: 'node index.js',
    workingDir: '.',
    installCommand: null,
    buildCommand: null,
    status: 'STOPPED',
    port: null,
    exitCode: null,
    failureReason: null,
    restartCount: 0,
    ...overrides,
  };
}

/** An API that crash-looped, and a web server that was stopped behind it. */
const afterACrash: ProcessSummary[] = [
  process({
    name: 'api',
    startOrder: 0,
    status: 'CRASHED',
    port: 3001,
    exitCode: 1,
    failureReason: 'ECONNREFUSED',
    restartCount: 5,
  }),
  process({ name: 'web', startOrder: 1, status: 'STOPPED', port: 5173 }),
];

describe('ProcessList', () => {
  it('names the process that failed, and why', () => {
    render(<ProcessList projectId="prj_1" processes={afterACrash} onRestart={() => {}} />);

    expect(dom.getByText('api')).toBeInTheDocument();
    expect(dom.getByText(/ECONNREFUSED/)).toBeInTheDocument();
    expect(dom.getByText(/exit 1/)).toBeInTheDocument();
  });

  it('shows the port each process listens on', () => {
    render(<ProcessList projectId="prj_1" processes={afterACrash} onRestart={() => {}} />);

    expect(dom.getByText('3001')).toBeInTheDocument();
    expect(dom.getByText('5173')).toBeInTheDocument();
  });

  it('reports a crash loop as a count rather than silently', () => {
    render(<ProcessList projectId="prj_1" processes={afterACrash} onRestart={() => {}} />);

    expect(dom.getByText(/Restarted 5 times/)).toBeInTheDocument();
  });

  it('restarts the process it names, not the project', async () => {
    const onRestart = vi.fn();
    render(<ProcessList projectId="prj_1" processes={afterACrash} onRestart={onRestart} />);

    const [restartApi] = dom.getAllByRole('button', { name: /restart/i });
    await userEvent.click(restartApi as HTMLElement);

    expect(onRestart).toHaveBeenCalledTimes(1);
    expect(onRestart).toHaveBeenCalledWith('api');
  });

  it('restarts the sibling when the sibling is the one asked about', async () => {
    const onRestart = vi.fn();
    render(<ProcessList projectId="prj_1" processes={afterACrash} onRestart={onRestart} />);

    const [, restartWeb] = dom.getAllByRole('button', { name: /restart/i });
    await userEvent.click(restartWeb as HTMLElement);

    expect(onRestart).toHaveBeenCalledWith('web');
  });

  /// A project with no processes cannot start, and the list is where that is
  /// visible before Start is pressed and refuses.
  it('says so when there is nothing to run', () => {
    render(<ProcessList projectId="prj_1" processes={[]} onRestart={() => {}} />);

    expect(dom.getByText(/no processes/i)).toBeInTheDocument();
    expect(dom.queryByRole('button')).not.toBeInTheDocument();
  });
});
