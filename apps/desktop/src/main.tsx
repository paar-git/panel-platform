import { StrictMode, lazy, Suspense } from 'react';
import { createRoot } from 'react-dom/client';

import CoreGate from './shell/CoreGate';
import LaunchScreen from './shell/LaunchScreen';
import './styles.css';

// Started immediately so the chunk loads while the launch screen is up, not
// after the core is ready. Monaco lives behind this import; pulling it into
// the first paint is what made the window sit empty after it appeared.
const app = import('./App');
const App = lazy(() => app);

const container = document.getElementById('root');
if (!container) {
  // Cannot happen with the shipped index.html, but failing loudly beats a
  // blank window that gives no clue what went wrong.
  throw new Error('the root element is missing from index.html');
}

createRoot(container).render(
  <StrictMode>
    <CoreGate>
      <Suspense fallback={<LaunchScreen />}>
        <App />
      </Suspense>
    </CoreGate>
  </StrictMode>,
);
