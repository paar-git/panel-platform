/**
 * The bar above every section.
 *
 * Search, then whatever is worth interrupting for. It is deliberately thin on
 * content: the sections carry their own headings, so a second title here would
 * say the same thing twice.
 *
 * The search field is a button rather than an input. It opens the command
 * palette, which is the thing that actually searches — an input that silently
 * hands over to a dialog on the first keystroke is a worse version of a button
 * that says so.
 */
import { Button, IconButton } from '../ui/primitives';

export default function TopBar({
  running,
  updateAvailable,
  onOpenPalette,
  onInstallUpdate,
  onOpenActivity,
  onRefresh,
}: {
  /** How many projects have something up. */
  running: number;
  /** The version on offer, or null when this build is current. */
  updateAvailable: string | null;
  onOpenPalette: () => void;
  onInstallUpdate: () => void;
  onOpenActivity: () => void;
  onRefresh: () => void;
}) {
  return (
    <header className="flex h-[52px] shrink-0 items-center gap-3 border-b border-edge px-4">
      <button
        type="button"
        onClick={onOpenPalette}
        className="flex h-[32px] w-full max-w-[520px] items-center gap-2.5 rounded-full border border-edge bg-raised px-3 text-left text-[12.5px] text-faint transition-colors hover:border-edge-strong"
      >
        <span aria-hidden>⌕</span>
        <span className="min-w-0 flex-1 truncate">Search projects, files and commands</span>
        <kbd className="rounded-[5px] bg-overlay px-1.5 py-0.5 font-mono text-[10.5px] text-faint">
          Ctrl K
        </kbd>
      </button>

      <div className="flex-1" />

      {updateAvailable !== null && (
        <Button variant="primary" size="sm" onClick={onInstallUpdate}>
          ↓ Update {updateAvailable}
        </Button>
      )}

      <span className="flex items-center gap-1.5 text-[12px] text-muted">
        <span
          aria-hidden
          className={`h-1.5 w-1.5 rounded-full ${running > 0 ? 'bg-ok' : 'bg-faint'}`}
        />
        {running} running
      </span>

      <IconButton icon="activity" label="Activity" onClick={onOpenActivity} />
      <IconButton icon="refresh" label="Refresh" onClick={onRefresh} />
    </header>
  );
}
