/**
 * What is on screen until the core is ready to talk.
 *
 * Painted with the same mark and charcoal as the rest of the product, so the
 * window that appears on click already looks like the application rather than
 * a blank WebView. The HTML copy in `index.html` covers the moment before
 * this component mounts.
 */
import Logo from '../ui/Logo';

export default function LaunchScreen({ error }: { error?: string | null }) {
  return (
    <div className="launch-screen" role="status" aria-live="polite">
      <div className="launch-screen__card">
        <div className="launch-screen__mark">
          <Logo size={56} title="Panel Platform" />
        </div>
        <p className="launch-screen__name">Panel Platform</p>
        {error ? (
          <p className="launch-screen__error">{error}</p>
        ) : (
          <>
            <p className="launch-screen__caption">Starting</p>
            <div className="launch-screen__bar" aria-hidden>
              <span />
            </div>
          </>
        )}
      </div>
    </div>
  );
}
