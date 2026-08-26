/**
 * The catalogue, assembled.
 *
 * One flat list in category order, plus the lookups the interface and the
 * stylesheet generator both read. Nothing here contains a colour: the palettes
 * live in `catalogue/`, ten to a file, so editing one theme means opening one
 * small file rather than a thousand-line register.
 */

import { CATEGORIES } from './categories';
import { CREATIVE_THEMES } from './catalogue/creative';
import { DEVELOPER_THEMES } from './catalogue/developer';
import { FUTURISTIC_THEMES } from './catalogue/futuristic';
import { GAMING_THEMES } from './catalogue/gaming';
import { HACKER_THEMES } from './catalogue/hacker';
import { MINIMAL_THEMES } from './catalogue/minimal';
import { NATURE_THEMES } from './catalogue/nature';
import { RETRO_THEMES } from './catalogue/retro';
import type { CategoryId, Theme } from './types';

const BY_CATEGORY: Record<CategoryId, readonly Theme[]> = {
  minimal: MINIMAL_THEMES,
  developer: DEVELOPER_THEMES,
  hacker: HACKER_THEMES,
  futuristic: FUTURISTIC_THEMES,
  gaming: GAMING_THEMES,
  nature: NATURE_THEMES,
  retro: RETRO_THEMES,
  creative: CREATIVE_THEMES,
};

/** Every theme, in the order the browser presents them. */
export const THEMES: readonly Theme[] = CATEGORIES.flatMap((category) => BY_CATEGORY[category.id]);

export const THEME_BY_ID: ReadonlyMap<string, Theme> = new Map(
  THEMES.map((theme) => [theme.id, theme]),
);

export function themesInCategory(category: CategoryId): readonly Theme[] {
  return BY_CATEGORY[category];
}

/** The theme a fresh installation opens on.
 *
 * `panel-warm` rather than `pure-dark`: it is the palette the application was
 * designed in, so it is what the screens were drawn against. The charcoal
 * default remains in the catalogue for anyone who prefers a neutral ground.
 */
export const DEFAULT_THEME_ID = 'panel-warm';

/**
 * What the five original ids became.
 *
 * `dark` and `light` were renamed when they joined a catalogue where "dark" is
 * an entire category; the other three kept theirs. This map is the only reason
 * an existing installation does not silently reset to the default on upgrade,
 * so an entry may be added but never removed.
 */
export const LEGACY_THEME_IDS: Readonly<Record<string, string>> = {
  dark: 'pure-dark',
  light: 'pure-light',
  amber: 'amber',
  midnight: 'midnight',
  nord: 'nord',
};

/** True for an id the catalogue can render right now. */
export function isThemeId(value: unknown): value is string {
  return typeof value === 'string' && THEME_BY_ID.has(value);
}

/**
 * Resolve anything that has ever been a valid theme id to one that is valid
 * now, or to the default.
 */
export function resolveThemeId(value: unknown): string {
  if (typeof value !== 'string') return DEFAULT_THEME_ID;
  if (THEME_BY_ID.has(value)) return value;

  const migrated = LEGACY_THEME_IDS[value];
  return migrated && THEME_BY_ID.has(migrated) ? migrated : DEFAULT_THEME_ID;
}

export { CATEGORIES, CATEGORY_BY_ID } from './categories';
export { measureTheme, renderAllThemes, renderTheme, resolveTheme } from './css';
export type { ResolvedTheme, ThemeContrast } from './css';
export * from './types';
