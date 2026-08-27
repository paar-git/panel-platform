/**
 * A dropdown that belongs to this application.
 *
 * The platform's own `<select>` opens a list drawn by the operating system: a
 * white menu in a dark application on Windows, a different shape on Linux, and
 * no way to show a description beside an option. This one is styled, keyboard
 * operable, and searchable once the list is long enough to need it.
 *
 * It is positioned with `fixed` and measured after mount, so a dropdown opened
 * near the bottom of the window flips upward and never renders past the edge.
 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';

import Icon from './Icon';

export interface SelectOption<T extends string> {
  value: T;
  label: string;
  description?: string;
  /** Shown greyed and not selectable, with the reason as a tooltip. */
  disabled?: boolean;
  reason?: string;
}

/** Long enough that scanning it beats reading it. */
const SEARCH_THRESHOLD = 8;

export default function Select<T extends string>({
  value,
  options,
  onChange,
  label,
  placeholder = 'Select…',
  hint,
  error,
  disabled,
  id,
}: {
  value: T | null;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  label?: string;
  placeholder?: string;
  hint?: string;
  error?: string;
  disabled?: boolean;
  id?: string;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [highlighted, setHighlighted] = useState(0);
  const [placement, setPlacement] = useState<{
    left: number;
    top: number;
    width: number;
    maxHeight: number;
  } | null>(null);

  const trigger = useRef<HTMLButtonElement | null>(null);
  const list = useRef<HTMLDivElement | null>(null);

  const selected = options.find((option) => option.value === value) ?? null;
  const searchable = options.length >= SEARCH_THRESHOLD;

  const matches = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle.length === 0) return options;
    return options.filter(
      (option) =>
        option.label.toLowerCase().includes(needle) ||
        option.description?.toLowerCase().includes(needle),
    );
  }, [options, query]);

  // Measured after mount rather than guessed: the height depends on how many
  // options there are, and one opened near the bottom of the window has to
  // come up instead of down.
  useLayoutEffect(() => {
    if (!open) {
      setPlacement(null);
      return;
    }
    const anchor = trigger.current?.getBoundingClientRect();
    if (!anchor) return;

    const margin = 8;
    const below = window.innerHeight - anchor.bottom - margin;
    const above = anchor.top - margin;
    const openUp = below < 200 && above > below;
    const maxHeight = Math.max(140, Math.min(320, openUp ? above : below));

    setPlacement({
      left: anchor.left,
      top: openUp ? anchor.top - maxHeight - 4 : anchor.bottom + 4,
      width: anchor.width,
      maxHeight,
    });
  }, [open, matches.length]);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: MouseEvent) {
      const target = event.target as Node;
      if (!list.current?.contains(target) && !trigger.current?.contains(target)) close();
    }
    // Closing on scroll rather than repositioning: the anchor may have moved
    // out of view entirely, and a menu floating over unrelated content is
    // worse than one that dismissed.
    window.addEventListener('mousedown', onPointerDown, true);
    window.addEventListener('resize', close);
    window.addEventListener('scroll', close, true);
    return () => {
      window.removeEventListener('mousedown', onPointerDown, true);
      window.removeEventListener('resize', close);
      window.removeEventListener('scroll', close, true);
    };
  }, [open]);

  useEffect(() => setHighlighted(0), [query]);

  useEffect(() => {
    if (!open) return;
    list.current
      ?.querySelector<HTMLElement>(`[data-index="${highlighted}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  }, [highlighted, open]);

  function close() {
    setOpen(false);
    setQuery('');
  }

  function choose(index: number) {
    const option = matches[index];
    if (!option || option.disabled) return;
    onChange(option.value);
    close();
    trigger.current?.focus();
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (!open) {
      if (event.key === 'Enter' || event.key === ' ' || event.key === 'ArrowDown') {
        event.preventDefault();
        setOpen(true);
      }
      return;
    }

    switch (event.key) {
      case 'Escape':
        event.preventDefault();
        close();
        trigger.current?.focus();
        break;
      case 'ArrowDown':
        event.preventDefault();
        setHighlighted((current) => (matches.length === 0 ? 0 : (current + 1) % matches.length));
        break;
      case 'ArrowUp':
        event.preventDefault();
        setHighlighted((current) =>
          matches.length === 0 ? 0 : (current - 1 + matches.length) % matches.length,
        );
        break;
      case 'Home':
        event.preventDefault();
        setHighlighted(0);
        break;
      case 'End':
        event.preventDefault();
        setHighlighted(Math.max(0, matches.length - 1));
        break;
      case 'Enter':
        event.preventDefault();
        choose(highlighted);
        break;
    }
  }

  return (
    <div className="block">
      {label && (
        <label htmlFor={id} className="mb-1.5 block text-[13px] text-ink">
          {label}
        </label>
      )}

      <button
        id={id}
        ref={trigger}
        type="button"
        role="combobox"
        aria-expanded={open}
        aria-haspopup="listbox"
        disabled={disabled}
        onClick={() => (open ? close() : setOpen(true))}
        onKeyDown={onKeyDown}
        className={`flex h-[30px] w-full items-center gap-2 rounded-[5px] border bg-canvas px-2.5 text-left text-[12.5px] disabled:pointer-events-none disabled:opacity-40 ${
          error ? 'border-danger' : open ? 'border-accent' : 'border-edge hover:border-edge-strong'
        }`}
      >
        <span className={`min-w-0 flex-1 truncate ${selected ? 'text-ink' : 'text-faint'}`}>
          {selected?.label ?? placeholder}
        </span>
        <Icon name="chevron-down" size={14} className="text-muted" />
      </button>

      {error ? (
        <span className="mt-1.5 flex items-start gap-1 text-[12px] text-danger">
          <Icon name="alert" size={13} className="mt-px" />
          {error}
        </span>
      ) : (
        hint && <span className="mt-1.5 block text-[12px] text-faint">{hint}</span>
      )}

      {open && (
        <div
          ref={list}
          role="listbox"
          style={{
            left: placement?.left,
            top: placement?.top,
            width: placement?.width,
            maxHeight: placement?.maxHeight,
            visibility: placement ? 'visible' : 'hidden',
          }}
          className="fixed z-[70] flex flex-col overflow-hidden rounded-[10px] border border-edge bg-overlay shadow-[0_12px_36px_rgba(0,0,0,0.5)]"
        >
          {searchable && (
            <div className="shrink-0 border-b border-edge p-2">
              <input
                autoFocus
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={onKeyDown}
                placeholder="Search…"
                aria-label="Search options"
                className="h-[26px] w-full rounded-[5px] border border-edge bg-canvas px-2 text-[12px] text-ink placeholder:text-faint select-text focus:border-accent"
              />
            </div>
          )}

          <div className="min-h-0 flex-1 overflow-y-auto p-1">
            {matches.length === 0 && (
              <p className="px-2 py-3 text-center text-[12px] text-faint">No match.</p>
            )}
            {matches.map((option, index) => {
              const isSelected = option.value === value;
              return (
                <button
                  key={option.value}
                  type="button"
                  role="option"
                  data-index={index}
                  aria-selected={isSelected}
                  disabled={option.disabled}
                  title={option.reason}
                  onMouseMove={() => setHighlighted(index)}
                  onClick={() => choose(index)}
                  className={`flex w-full items-start gap-2 rounded-[6px] px-2 py-1.5 text-left disabled:opacity-40 ${
                    index === highlighted && !option.disabled ? 'bg-raised' : ''
                  }`}
                >
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px] text-ink">{option.label}</span>
                    {option.description && (
                      <span className="block truncate text-[12px] text-muted">
                        {option.description}
                      </span>
                    )}
                  </span>
                  {isSelected && (
                    <span className="mt-0.5 text-accent">
                      <Icon name="check" size={14} />
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
