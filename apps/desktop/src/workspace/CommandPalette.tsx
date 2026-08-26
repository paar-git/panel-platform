/**
 * The palette, in both of its modes.
 *
 * Opened with Ctrl+Shift+P it lists commands; opened with Ctrl+P it lists
 * files, and typing `>` switches to commands — the same one widget with the
 * same one prompt, which is what makes the two shortcuts feel like one feature.
 *
 * Every command in it comes from the workspace's own registry, so there is
 * nothing here that looks like it works and does not.
 */
import { useEffect, useMemo, useRef, useState } from 'react';

import Icon from './Icon';
import { fileIconColor } from './fileIcons';
import { matchCommands, matchPaths, type Command } from './commands';
import { tabLabel, parentOf } from './tabs';

export type PaletteMode = 'commands' | 'files';

export default function CommandPalette({
  mode,
  commands,
  paths,
  loadingPaths,
  onFileQuery,
  onRunCommand,
  onOpenPath,
  onClose,
}: {
  mode: PaletteMode;
  commands: Command[];
  /**
   * Candidates for quick open. The core matches names as a substring; the
   * ranking below is what turns that into an ordered list.
   */
  paths: string[];
  loadingPaths: boolean;
  /** Raised as the file-mode term changes, so the workspace can go and search. */
  onFileQuery: (term: string) => void;
  onRunCommand: (command: Command) => void;
  onOpenPath: (path: string) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState(mode === 'commands' ? '>' : '');
  const [index, setIndex] = useState(0);
  const list = useRef<HTMLUListElement | null>(null);

  const showingCommands = query.startsWith('>');
  const term = showingCommands ? query.slice(1) : query;

  const commandMatches = useMemo(
    () => (showingCommands ? matchCommands(commands, term) : []),
    [showingCommands, commands, term],
  );
  const pathMatches = useMemo(
    () => (showingCommands ? [] : matchPaths(paths, term)),
    [showingCommands, paths, term],
  );
  const count = showingCommands ? commandMatches.length : pathMatches.length;

  // A new query means a new list; keeping the old index would leave the
  // highlight on whatever happens to be in that position now.
  useEffect(() => setIndex(0), [query]);

  // File mode asks the core, because it is the core that can walk the project.
  useEffect(() => {
    if (!showingCommands) onFileQuery(term);
  }, [showingCommands, term, onFileQuery]);

  useEffect(() => {
    list.current
      ?.querySelector<HTMLElement>(`[data-index="${index}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  }, [index]);

  function accept(at: number) {
    if (showingCommands) {
      const match = commandMatches[at];
      if (!match || match.item.enabled === false) return;
      onClose();
      onRunCommand(match.item);
      return;
    }
    const match = pathMatches[at];
    if (!match) return;
    onClose();
    onOpenPath(match.item);
  }

  function onKeyDown(event: React.KeyboardEvent) {
    switch (event.key) {
      case 'Escape':
        event.preventDefault();
        onClose();
        break;
      case 'ArrowDown':
        event.preventDefault();
        setIndex((current) => (count === 0 ? 0 : (current + 1) % count));
        break;
      case 'ArrowUp':
        event.preventDefault();
        setIndex((current) => (count === 0 ? 0 : (current - 1 + count) % count));
        break;
      case 'Enter':
        event.preventDefault();
        accept(index);
        break;
    }
  }

  return (
    <div
      className="fixed inset-0 z-40 flex justify-center pt-[10vh]"
      // A click anywhere outside dismisses. The backdrop is transparent —
      // VS Code does not dim the window behind its palette.
      onMouseDown={onClose}
    >
      <div
        onMouseDown={(event) => event.stopPropagation()}
        className="flex max-h-[60vh] w-[min(680px,90vw)] flex-col border border-vs-border bg-[#12182a] shadow-[0_12px_40px_rgba(0,0,0,0.6)]"
      >
        <div className="p-2.5">
          <input
            autoFocus
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={onKeyDown}
            aria-label={showingCommands ? 'Run a command' : 'Open a file by name'}
            placeholder={
              showingCommands ? 'Type a command name' : 'Type a file name (or > for commands)'
            }
            className="h-[26px] w-full border border-accent bg-vs-editor px-2 text-[13px] text-vs-text outline-none select-text"
          />
        </div>

        <ul ref={list} className="min-h-0 flex-1 overflow-y-auto pb-1">
          {showingCommands
            ? commandMatches.map((match, at) => (
                <li key={match.item.id} data-index={at}>
                  <button
                    type="button"
                    disabled={match.item.enabled === false}
                    onMouseMove={() => setIndex(at)}
                    onClick={() => accept(at)}
                    title={match.item.reason}
                    className={`flex w-full items-center gap-2 px-3 py-1 text-left text-[13px] ${
                      at === index ? 'bg-accent text-canvas' : 'text-vs-text'
                    } disabled:cursor-default disabled:opacity-40`}
                  >
                    <span className={at === index ? 'text-white/70' : 'text-vs-dim'}>
                      {match.item.category}:
                    </span>
                    <span className="flex-1 truncate">{match.item.title}</span>
                    {match.item.keybinding && (
                      <span className={at === index ? 'text-white/70' : 'text-vs-dim'}>
                        {match.item.keybinding}
                      </span>
                    )}
                  </button>
                </li>
              ))
            : pathMatches.map((match, at) => (
                <li key={match.item} data-index={at}>
                  <button
                    type="button"
                    onMouseMove={() => setIndex(at)}
                    onClick={() => accept(at)}
                    className={`flex w-full items-center gap-2 px-3 py-1 text-left text-[13px] ${
                      at === index ? 'bg-accent text-canvas' : 'text-vs-text'
                    }`}
                  >
                    <span style={{ color: fileIconColor(tabLabel(match.item)) }}>
                      <Icon name="file" size={15} />
                    </span>
                    <span className="truncate">{tabLabel(match.item)}</span>
                    <span
                      className={`min-w-0 flex-1 truncate ${
                        at === index ? 'text-white/70' : 'text-vs-dim'
                      }`}
                    >
                      {parentOf(match.item)}
                    </span>
                  </button>
                </li>
              ))}

          {count === 0 && (
            <li className="px-3 py-2 text-[13px] text-vs-dim">
              {showingCommands
                ? 'No matching commands.'
                : loadingPaths
                  ? 'Reading the project…'
                  : 'No matching files.'}
            </li>
          )}
        </ul>
      </div>
    </div>
  );
}
