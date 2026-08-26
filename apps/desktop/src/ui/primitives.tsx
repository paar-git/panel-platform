/**
 * The pieces every screen is built from.
 *
 * One definition each, so a card looks the same on the dashboard as it does in
 * settings and a change to how a button reads happens once. Before this, each
 * screen carried its own card, its own padding and its own idea of how big a
 * heading should be, which is how four screens ended up with four looks.
 *
 * Everything here is presentational and knows nothing about projects, Docker or
 * the core.
 */
import type { ReactNode } from 'react';

import Icon, { type IconName } from './Icon';

// ------------------------------------------------------------------- layout

/**
 * A screen's frame: a header with a title, a sentence of explanation and at
 * most one primary action, then the content.
 *
 * The width is capped because a settings row stretched across an ultrawide
 * monitor puts its label and its value too far apart to associate.
 */
export function PageShell({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="mx-auto w-full max-w-[1200px] px-8 py-6">
      <header className="mb-6 flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0">
          <h1 className="text-[28px] leading-tight font-semibold tracking-[-0.025em]">{title}</h1>
          {description && <p className="mt-0.5 text-[13px] text-muted">{description}</p>}
        </div>
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </header>
      {children}
    </div>
  );
}

/** A surface. The default container for anything grouped. */
export function Card({
  children,
  className = '',
  interactive,
}: {
  children: ReactNode;
  className?: string;
  interactive?: boolean;
}) {
  return (
    <div
      className={`rounded-[12px] border border-edge bg-surface ${
        interactive ? 'transition-colors hover:border-edge-strong' : ''
      } ${className}`}
    >
      {children}
    </div>
  );
}

/** A card's own header: a title, optional subtitle, optional actions. */
export function CardHeader({
  title,
  subtitle,
  actions,
}: {
  title: string;
  subtitle?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-4 border-b border-edge px-4 py-[13px]">
      <div className="min-w-0">
        <h2 className="text-[13.5px] font-semibold">{title}</h2>
        {subtitle && <p className="mt-px text-[12px] text-faint">{subtitle}</p>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
  );
}

/** A label-and-value line. The unit the detail screens are built from. */
export function DataRow({
  label,
  value,
  mono,
  hint,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
  hint?: string;
}) {
  return (
    <div className="flex items-baseline justify-between gap-6 border-b border-edge/60 px-4 py-[9px] last:border-b-0">
      <span className="shrink-0 text-[13px] text-muted" title={hint}>
        {label}
      </span>
      <span
        className={`min-w-0 truncate text-right text-[13px] text-ink ${
          mono ? 'font-mono text-[12px] select-text' : ''
        }`}
      >
        {value}
      </span>
    </div>
  );
}

// ------------------------------------------------------------------ controls

type ButtonVariant = 'primary' | 'default' | 'ghost' | 'danger';

export function Button({
  children,
  onClick,
  variant = 'default',
  size = 'md',
  disabled,
  title,
  type = 'button',
  icon,
  full,
  pending,
}: {
  children?: ReactNode;
  onClick?: () => void;
  variant?: ButtonVariant;
  size?: 'sm' | 'md';
  disabled?: boolean;
  title?: string;
  type?: 'button' | 'submit';
  icon?: IconName;
  full?: boolean;
  /** In flight. Swaps the icon for a spinner and blocks a second press —
   *  the button reports the wait rather than looking ignored. */
  pending?: boolean;
}) {
  const variants: Record<ButtonVariant, string> = {
    // The primary reads as a lit pill: the accent carries it, and the label is
    // the canvas colour rather than white, so it stays legible under a light
    // accent as well as a dark one.
    primary: 'bg-accent text-canvas font-semibold hover:brightness-110',
    default: 'border border-edge-strong bg-transparent text-ink hover:border-faint',
    ghost: 'text-muted hover:bg-raised hover:text-ink',
    danger: 'bg-danger/15 text-danger hover:bg-danger/25',
  };

  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled || pending}
      title={title}
      aria-busy={pending}
      className={`inline-flex shrink-0 items-center justify-center gap-1.5 rounded-full font-medium transition-[background-color,border-color,filter] duration-100 disabled:pointer-events-none disabled:opacity-40 ${
        size === 'sm' ? 'h-[28px] px-3 text-[12px]' : 'h-[32px] px-4 text-[12.5px]'
      } ${full ? 'w-full' : ''} ${variants[variant]}`}
    >
      {pending ? (
        <Icon name="refresh" size={size === 'sm' ? 13 : 14} className="animate-spin-slow" />
      ) : (
        icon && <Icon name={icon} size={size === 'sm' ? 13 : 14} />
      )}
      {children}
    </button>
  );
}

/** A square button that is only an icon. Always carries a label for both. */
export function IconButton({
  icon,
  label,
  onClick,
  disabled,
  active,
  size = 'md',
}: {
  icon: IconName;
  label: string;
  /** Given the event, so a caller can anchor a menu to the button. */
  onClick?: (event: React.MouseEvent<HTMLButtonElement>) => void;
  disabled?: boolean;
  active?: boolean;
  size?: 'sm' | 'md';
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={label}
      aria-label={label}
      aria-pressed={active}
      className={`inline-grid shrink-0 place-items-center rounded-full disabled:pointer-events-none disabled:opacity-40 ${
        size === 'sm' ? 'h-[26px] w-[26px]' : 'h-[28px] w-[28px]'
      } ${active ? 'bg-raised text-ink' : 'text-muted hover:bg-raised hover:text-ink'}`}
    >
      <Icon name={icon} size={size === 'sm' ? 14 : 16} />
    </button>
  );
}

export function TextInput({
  value,
  onChange,
  placeholder,
  label,
  hint,
  error,
  mono,
  type = 'text',
  autoFocus,
  maxLength,
  onKeyDown,
  id,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  label?: string;
  hint?: string;
  error?: string;
  mono?: boolean;
  type?: 'text' | 'password' | 'number';
  autoFocus?: boolean;
  maxLength?: number;
  onKeyDown?: (event: React.KeyboardEvent<HTMLInputElement>) => void;
  id?: string;
}) {
  return (
    <label className="block" htmlFor={id}>
      {label && <span className="mb-1.5 block text-[13px] text-ink">{label}</span>}
      <input
        id={id}
        type={type}
        value={value}
        autoFocus={autoFocus}
        maxLength={maxLength}
        spellCheck={false}
        placeholder={placeholder}
        onKeyDown={onKeyDown}
        onChange={(event) => onChange(event.target.value)}
        aria-invalid={error ? true : undefined}
        className={`h-[30px] w-full rounded-[5px] border bg-canvas px-2.5 text-[12.5px] text-ink placeholder:text-faint select-text focus:border-accent ${
          error ? 'border-danger' : 'border-edge'
        } ${mono ? 'font-mono text-[12px]' : ''}`}
      />
      {/* The error replaces the hint rather than pushing it down: two lines of
          helper text under one field is one too many. */}
      {error ? (
        <span className="mt-1.5 flex items-start gap-1 text-[12px] text-danger">
          <Icon name="alert" size={13} className="mt-px" />
          {error}
        </span>
      ) : (
        hint && <span className="mt-1.5 block text-[12px] text-faint">{hint}</span>
      )}
    </label>
  );
}

/** A switch. Used wherever a setting is genuinely on or off. */
export function Toggle({
  checked,
  onChange,
  label,
  description,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-6 border-b border-edge/60 py-2.5 last:border-b-0">
      <div className="min-w-0">
        <p className="text-[13px] text-ink">{label}</p>
        {description && <p className="mt-0.5 text-[12px] text-muted">{description}</p>}
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative mt-0.5 h-5 w-9 shrink-0 rounded-full disabled:pointer-events-none disabled:opacity-40 ${
          checked ? 'bg-accent' : 'bg-edge-strong'
        }`}
      >
        <span
          className={`absolute top-0.5 h-4 w-4 rounded-full bg-white transition-[left] duration-160 ${
            checked ? 'left-[18px]' : 'left-0.5'
          }`}
        />
      </button>
    </div>
  );
}

// -------------------------------------------------------------------- status

export type Tone = 'ok' | 'warn' | 'danger' | 'accent' | 'neutral';

const TONES: Record<Tone, { dot: string; badge: string }> = {
  ok: { dot: 'bg-ok', badge: 'bg-ok-soft text-ok' },
  warn: { dot: 'bg-warn', badge: 'bg-warn-soft text-warn' },
  danger: { dot: 'bg-danger', badge: 'bg-danger-soft text-danger' },
  accent: { dot: 'bg-accent', badge: 'bg-accent-soft text-accent' },
  neutral: { dot: 'bg-faint', badge: 'bg-raised text-muted' },
};

export function Badge({
  children,
  tone = 'neutral',
  dot,
  title,
}: {
  children: ReactNode;
  tone?: Tone;
  dot?: boolean;
  /** Hover text. A badge that is a single word sometimes needs a sentence. */
  title?: string;
}) {
  return (
    <span
      title={title}
      className={`inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium ${TONES[tone].badge}`}
    >
      {dot && <span className={`h-1.5 w-1.5 rounded-full ${TONES[tone].dot}`} aria-hidden />}
      {children}
    </span>
  );
}

/**
 * A proportion, drawn.
 *
 * `value` is a percentage. A bar whose number nobody measured is worse than no
 * bar, so callers pass `unknown` rather than zero when there is nothing to
 * show.
 */
export function Meter({
  value,
  label,
  caption,
  tone,
  unknown,
}: {
  value: number;
  label: string;
  caption?: string;
  tone?: Tone;
  unknown?: boolean;
}) {
  const clamped = Math.max(0, Math.min(100, value));
  // Colour by pressure rather than by caller preference: anything above 90% is
  // worth noticing wherever it appears.
  const derived: Tone = tone ?? (clamped >= 90 ? 'danger' : clamped >= 75 ? 'warn' : 'accent');

  return (
    <div>
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-[12px] text-muted">{label}</span>
        <span className="tabular text-[12px] text-ink">
          {unknown ? 'not measured' : `${Math.round(clamped)}%`}
        </span>
      </div>
      <div className="mt-[7px] h-[5px] overflow-hidden rounded-full bg-edge">
        {!unknown && (
          <div
            className={`h-full rounded-full transition-[width] duration-300 ${TONES[derived].dot}`}
            style={{ width: `${clamped}%` }}
          />
        )}
      </div>
      {caption && <p className="tabular mt-1.5 truncate text-[11px] text-faint">{caption}</p>}
    </div>
  );
}

/** A single number worth glancing at. */
export function Stat({
  label,
  value,
  tone,
  hint,
  onClick,
}: {
  label: string;
  value: ReactNode;
  tone?: Tone;
  hint?: string;
  onClick?: () => void;
}) {
  const colour =
    tone === 'ok'
      ? 'text-ok'
      : tone === 'warn'
        ? 'text-warn'
        : tone === 'danger'
          ? 'text-danger'
          : 'text-ink';

  const body = (
    <>
      <p className="text-[11px] font-semibold tracking-[0.08em] text-faint uppercase">{label}</p>
      <p
        className={`tabular mt-1.5 text-[30px] leading-none font-semibold tracking-tight ${colour}`}
      >
        {value}
      </p>
    </>
  );

  if (onClick) {
    return (
      <button
        type="button"
        onClick={onClick}
        title={hint}
        className="rounded-[12px] border border-edge bg-surface px-4 py-3.5 text-left transition-colors hover:border-edge-strong hover:bg-raised"
      >
        {body}
      </button>
    );
  }
  return (
    <div title={hint} className="rounded-[12px] border border-edge bg-surface px-4 py-3.5">
      {body}
    </div>
  );
}

// ---------------------------------------------------------------- feedback

/** A shape held while the real content loads. */
export function Skeleton({ className = '' }: { className?: string }) {
  return <div aria-hidden className={`skeleton rounded-[5px] ${className}`} />;
}

/**
 * Nothing here — and what to do about it.
 *
 * Always takes at least one action. An empty state that only explains why it is
 * empty leaves the user to work out the next step themselves.
 */
export function EmptyState({
  icon,
  title,
  description,
  actions,
  children,
}: {
  icon: IconName;
  title: string;
  description: string;
  actions?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center px-6 py-12 text-center">
      <span className="mb-3 grid h-10 w-10 place-items-center rounded-[7px] border border-edge bg-raised text-muted">
        <Icon name={icon} size={22} />
      </span>
      <h3 className="text-[15px] font-medium">{title}</h3>
      <p className="mt-1 max-w-sm text-[13px] leading-relaxed text-muted">{description}</p>
      {actions && <div className="mt-5 flex flex-wrap justify-center gap-2">{actions}</div>}
      {children}
    </div>
  );
}

/**
 * A short, actionable message.
 *
 * Deliberately built around a heading and a row of buttons rather than a
 * paragraph: "Docker is not running" with a Retry button next to it is read;
 * four sentences explaining what Docker is are not.
 */
export function Banner({
  tone,
  title,
  description,
  actions,
  onDismiss,
}: {
  tone: Tone;
  title: string;
  description?: string;
  actions?: ReactNode;
  onDismiss?: () => void;
}) {
  const edge =
    tone === 'warn'
      ? 'border-warn/30 bg-warn-soft'
      : tone === 'danger'
        ? 'border-danger/30 bg-danger-soft'
        : tone === 'ok'
          ? 'border-ok/30 bg-ok-soft'
          : 'border-edge bg-raised';

  return (
    <div
      className={`flex flex-wrap items-center gap-x-3 gap-y-2 rounded-[7px] border px-3 py-2.5 ${edge}`}
    >
      <span className={`h-2 w-2 shrink-0 rounded-full ${TONES[tone].dot}`} aria-hidden />
      <div className="min-w-[200px] flex-1">
        <p className="text-[13px] font-medium text-ink">{title}</p>
        {description && <p className="mt-0.5 text-[12px] text-muted">{description}</p>}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
      {onDismiss && <IconButton icon="close" label="Dismiss" size="sm" onClick={onDismiss} />}
    </div>
  );
}

/** A horizontal tab strip. */
export function Tabs<T extends string>({
  tabs,
  active,
  onSelect,
}: {
  tabs: { id: T; label: string; badge?: number }[];
  active: T;
  onSelect: (id: T) => void;
}) {
  return (
    <div role="tablist" className="flex gap-1 overflow-x-auto border-b border-edge">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          role="tab"
          type="button"
          aria-selected={active === tab.id}
          onClick={() => onSelect(tab.id)}
          className={`-mb-px flex shrink-0 items-center gap-1.5 border-b-2 px-3 py-2 text-[13px] ${
            active === tab.id
              ? 'border-accent text-ink'
              : 'border-transparent text-muted hover:text-ink'
          }`}
        >
          {tab.label}
          {tab.badge !== undefined && tab.badge > 0 && (
            <span className="tabular rounded-full bg-raised px-1.5 text-[11px] text-muted">
              {tab.badge}
            </span>
          )}
        </button>
      ))}
    </div>
  );
}
