/**
 * The one menu widget.
 *
 * Right-clicking a file, right-clicking a tab and opening a top menu all show
 * the same thing, so they are the same component: one set of keyboard rules,
 * one dismissal rule, one place where a menu that would hang off the edge of
 * the window gets flipped back on.
 */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';

import Icon, { type IconName } from './Icon';

export interface MenuAction {
  id: string;
  label: string;
  icon?: IconName;
  /** Shown greyed on the right. Display only — the shortcut itself is global. */
  keybinding?: string;
  enabled?: boolean;
  /** Renders in red. Used for delete. */
  danger?: boolean;
  run: () => void;
}

/** A horizontal rule between groups. */
export interface MenuSeparator {
  id: string;
  separator: true;
}

export type MenuEntry = MenuAction | MenuSeparator;

export function isSeparator(entry: MenuEntry): entry is MenuSeparator {
  return 'separator' in entry;
}

export interface MenuPosition {
  x: number;
  y: number;
  /**
   * Anchor the menu's top-left below this point rather than at it — what a
   * dropdown from a menu-bar button wants and a right-click does not.
   */
  below?: boolean;
}

export default function ContextMenu({
  entries,
  position,
  onClose,
}: {
  entries: MenuEntry[];
  position: MenuPosition;
  onClose: () => void;
}) {
  const menu = useRef<HTMLDivElement | null>(null);
  const [placement, setPlacement] = useState<{ left: number; top: number } | null>(null);
  const [focused, setFocused] = useState<number>(() => firstEnabled(entries, 0, 1));

  // Measured after mount rather than guessed: the height depends on how many
  // items there are, and a menu opened near the bottom of the window has to
  // come up instead of down.
  useLayoutEffect(() => {
    const element = menu.current;
    if (!element) return;

    const { width, height } = element.getBoundingClientRect();
    const margin = 6;
    const left = Math.max(margin, Math.min(position.x, window.innerWidth - width - margin));
    const preferred = position.y;
    const top =
      preferred + height + margin > window.innerHeight
        ? Math.max(margin, preferred - (position.below ? 0 : height))
        : preferred;
    setPlacement({
      left,
      top: Math.max(margin, Math.min(top, window.innerHeight - height - margin)),
    });
  }, [entries, position]);

  useEffect(() => {
    function onPointerDown(event: MouseEvent) {
      if (!menu.current?.contains(event.target as Node)) onClose();
    }
    // `capture` so the click that dismisses a menu does not also activate
    // whatever it was over.
    window.addEventListener('mousedown', onPointerDown, true);
    window.addEventListener('resize', onClose);
    window.addEventListener('blur', onClose);
    return () => {
      window.removeEventListener('mousedown', onPointerDown, true);
      window.removeEventListener('resize', onClose);
      window.removeEventListener('blur', onClose);
    };
  }, [onClose]);

  function onKeyDown(event: React.KeyboardEvent) {
    switch (event.key) {
      case 'Escape':
        event.preventDefault();
        onClose();
        break;
      case 'ArrowDown':
        event.preventDefault();
        setFocused((current) => firstEnabled(entries, current + 1, 1));
        break;
      case 'ArrowUp':
        event.preventDefault();
        setFocused((current) => firstEnabled(entries, current - 1, -1));
        break;
      case 'Home':
        event.preventDefault();
        setFocused(firstEnabled(entries, 0, 1));
        break;
      case 'End':
        event.preventDefault();
        setFocused(firstEnabled(entries, entries.length - 1, -1));
        break;
      case 'Enter':
      case ' ': {
        event.preventDefault();
        const entry = entries[focused];
        if (entry && !isSeparator(entry) && entry.enabled !== false) {
          onClose();
          entry.run();
        }
        break;
      }
    }
  }

  return (
    <div
      ref={menu}
      role="menu"
      tabIndex={-1}
      autoFocus
      onKeyDown={onKeyDown}
      style={{
        left: placement?.left ?? position.x,
        top: placement?.top ?? position.y,
        // Hidden for the single frame between mounting and being measured, so
        // the menu never appears in the wrong place and jumps.
        visibility: placement ? 'visible' : 'hidden',
      }}
      className="fixed z-50 min-w-56 border border-vs-border bg-[#12182a] py-1 text-[13px] shadow-[0_8px_24px_rgba(0,0,0,0.55)] outline-none"
    >
      {entries.map((entry, index) =>
        isSeparator(entry) ? (
          <div key={entry.id} role="separator" className="my-1 h-px bg-vs-border" />
        ) : (
          <button
            key={entry.id}
            type="button"
            role="menuitem"
            disabled={entry.enabled === false}
            onMouseEnter={() => setFocused(index)}
            onClick={() => {
              onClose();
              entry.run();
            }}
            className={`flex w-full items-center gap-2.5 px-3 py-1 text-left ${
              entry.danger ? 'text-red-300' : 'text-vs-text'
            } ${index === focused ? 'bg-accent text-canvas' : ''} disabled:cursor-default disabled:text-vs-dim disabled:opacity-50 ${
              index === focused && entry.enabled === false ? 'bg-transparent' : ''
            }`}
          >
            <span className="w-4 text-center opacity-80">
              {entry.icon && <Icon name={entry.icon} size={14} />}
            </span>
            <span className="flex-1 truncate">{entry.label}</span>
            {entry.keybinding && (
              <span className={index === focused ? 'text-white/70' : 'text-vs-dim'}>
                {entry.keybinding}
              </span>
            )}
          </button>
        ),
      )}
    </div>
  );
}

/**
 * The next entry that can be focused, wrapping at both ends.
 *
 * Separators and disabled items are skipped rather than focused-and-inert,
 * which is what makes holding the down arrow feel like a menu rather than a
 * list with holes in it.
 */
function firstEnabled(entries: MenuEntry[], from: number, step: number): number {
  if (entries.length === 0) return 0;
  for (let attempt = 0; attempt < entries.length; attempt += 1) {
    const index = (((from + step * attempt) % entries.length) + entries.length) % entries.length;
    const entry = entries[index];
    if (entry && !isSeparator(entry) && entry.enabled !== false) return index;
  }
  return 0;
}
