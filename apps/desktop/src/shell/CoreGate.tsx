/**
 * Holds the window on the launch screen until the core is ready.
 *
 * The window itself is already open: this is the page inside it. Commands that
 * need `AppState` fail if they run before that state exists, so children are
 * not mounted until `core://ready`.
 */
import { useEffect, useState, type ReactNode } from 'react';

import { launchStatus, onCoreFailed, onCoreReady, type LaunchStatus } from '../api';
import LaunchScreen from './LaunchScreen';

function hold(previous: LaunchStatus, next: LaunchStatus): LaunchStatus {
  // A late "starting" must not overwrite a ready or failed answer: the status
  // command and the event race, and the slower one used to flash the splash
  // after the application was already up.
  if (previous.state === 'ready' || previous.state === 'failed') return previous;
  return next;
}

export default function CoreGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<LaunchStatus>({ state: 'starting' });

  useEffect(() => {
    let cancelled = false;
    const ask = () =>
      launchStatus()
        .then((next) => {
          if (!cancelled) setStatus((previous) => hold(previous, next));
        })
        .catch(() => {
          // No bridge yet (tests, or the window beat the IPC). Stay on
          // starting; the events still settle it.
        });

    const ready = onCoreReady(() => {
      if (!cancelled) setStatus((previous) => hold(previous, { state: 'ready' }));
    });
    const failed = onCoreFailed((message) => {
      if (!cancelled) setStatus((previous) => hold(previous, { state: 'failed', message }));
    });
    void ask();
    // Asked a second time once both listeners are actually registered.
    // Registration is asynchronous, so a core that became ready after the
    // first answer was taken but before `listen` was in place would emit
    // `core://ready` into nothing, and the window would sit on the launch
    // screen for ever. `hold` makes the extra answer harmless when the event
    // did arrive.
    void Promise.all([ready, failed]).then(ask);
    return () => {
      cancelled = true;
      void ready.then((stop) => stop());
      void failed.then((stop) => stop());
    };
  }, []);

  if (status.state === 'ready') return children;
  return <LaunchScreen error={status.state === 'failed' ? status.message : null} />;
}

export { hold as holdLaunchStatus };
