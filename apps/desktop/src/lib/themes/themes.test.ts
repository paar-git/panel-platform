import { describe, expect, it } from 'vitest';

import { CATEGORIES } from './categories';
import { contrastRounded, parseHex } from './color';
import { measureTheme, resolveTheme } from './css';
import { DEFAULT_THEME_ID, LEGACY_THEME_IDS, THEMES, THEME_BY_ID, resolveThemeId } from './index';
import { CANVAS_EFFECTS, CSS_EFFECTS } from './types';

const EFFECTS = new Set<string>([...CSS_EFFECTS, ...CANVAS_EFFECTS]);

/** The floor everything is held to unless the theme asks for stricter. */
const BODY = 4.5;
const LARGE = 3;

describe('the catalogue', () => {
  it('has a theme for every category, and no empty groups', () => {
    for (const category of CATEGORIES) {
      const inGroup = THEMES.filter((theme) => theme.category === category.id);
      expect(inGroup.length, `${category.id} is empty`).toBeGreaterThan(0);
    }
  });

  it('gives every category the ten it promises', () => {
    for (const category of CATEGORIES) {
      const inGroup = THEMES.filter((theme) => theme.category === category.id);
      // Minimal carries the extras kept for migration. Developer carries one
      // more than ten because `panel-warm` is the application's own palette
      // rather than a borrowed one — it is the default a fresh installation
      // opens on, and it lives in the catalogue so that a user who wanders off
      // it can find their way back.
      const extras: Partial<Record<string, number>> = { minimal: 11, developer: 11 };
      const expected = extras[category.id] ?? 10;
      expect(inGroup.length, `${category.id} has ${inGroup.length}`).toBe(expected);
    }
  });

  it('has no duplicate ids', () => {
    const seen = new Map<string, number>();
    for (const theme of THEMES) seen.set(theme.id, (seen.get(theme.id) ?? 0) + 1);

    const duplicates = [...seen.entries()].filter(([, count]) => count > 1);
    expect(duplicates).toEqual([]);
  });

  /** Ids are written into `localStorage` and into a CSS attribute selector, so
   *  anything but lower-case kebab is a bug waiting for a quoting mistake. */
  it('uses kebab-case ids', () => {
    for (const theme of THEMES) {
      expect(theme.id, theme.name).toMatch(/^[a-z0-9]+(-[a-z0-9]+)*$/);
    }
  });

  it('names every theme and explains it in one line', () => {
    for (const theme of THEMES) {
      expect(theme.name.length, theme.id).toBeGreaterThan(0);
      expect(theme.detail.length, theme.id).toBeGreaterThan(10);
    }
  });

  it('only asks for effects that exist', () => {
    for (const theme of THEMES) {
      if (theme.effect)
        expect(EFFECTS.has(theme.effect), `${theme.id}: ${theme.effect}`).toBe(true);
    }
  });

  it('credits every borrowed palette completely', () => {
    for (const theme of THEMES) {
      if (!theme.credit) continue;
      expect(theme.credit.work.length, theme.id).toBeGreaterThan(0);
      expect(theme.credit.author.length, theme.id).toBeGreaterThan(0);
      expect(theme.credit.licence.length, theme.id).toBeGreaterThan(0);
      expect(theme.credit.url, theme.id).toMatch(/^https:\/\//);
    }
  });

  it('states colours that are colours', () => {
    for (const theme of THEMES) {
      const { colour } = resolveTheme(theme);
      for (const [name, value] of Object.entries(colour)) {
        expect(() => parseHex(value), `${theme.id} ${name} = ${value}`).not.toThrow();
      }
    }
  });

  it('opens on a theme that exists', () => {
    expect(THEME_BY_ID.has(DEFAULT_THEME_ID)).toBe(true);
  });
});

describe('migration from the original five', () => {
  it('lands every original id on a theme that exists', () => {
    for (const [old, next] of Object.entries(LEGACY_THEME_IDS)) {
      expect(THEME_BY_ID.has(next), `${old} → ${next}`).toBe(true);
      expect(resolveThemeId(old)).toBe(next);
    }
  });

  it('keeps the three ids that did not change', () => {
    expect(resolveThemeId('amber')).toBe('amber');
    expect(resolveThemeId('midnight')).toBe('midnight');
    expect(resolveThemeId('nord')).toBe('nord');
  });

  it('falls back rather than writing an unknown id to the DOM', () => {
    expect(resolveThemeId('a-theme-that-never-existed')).toBe(DEFAULT_THEME_ID);
    expect(resolveThemeId(undefined)).toBe(DEFAULT_THEME_ID);
    expect(resolveThemeId(42)).toBe(DEFAULT_THEME_ID);
  });
});

/**
 * The readability gate.
 *
 * Eighty palettes is eighty chances to ship text nobody can read, and no
 * reviewer is going to open all of them. Each expectation names the theme and
 * quotes the measurement, so a failure says what to change rather than that
 * something is wrong.
 */
describe('contrast', () => {
  for (const theme of THEMES) {
    const target = theme.contrastTarget ?? BODY;
    const largeTarget = theme.contrastTarget ? theme.contrastTarget - 1.5 : LARGE;

    describe(`${theme.name} (${theme.id})`, () => {
      const m = measureTheme(theme);
      const round = (n: number) => Math.round(n * 100) / 100;

      it(`reads body text on every surface (needs ${target}:1)`, () => {
        expect(round(m.inkOnCanvas), 'ink on canvas').toBeGreaterThanOrEqual(target);
        expect(round(m.inkOnSurface), 'ink on surface').toBeGreaterThanOrEqual(target);
        expect(round(m.inkOnRaised), 'ink on raised').toBeGreaterThanOrEqual(target);
      });

      it(`reads secondary text (needs ${target}:1)`, () => {
        expect(round(m.mutedOnCanvas), 'muted on canvas').toBeGreaterThanOrEqual(target);
        expect(round(m.mutedOnSurface), 'muted on surface').toBeGreaterThanOrEqual(target);
      });

      it(`reads the quietest text (needs ${largeTarget}:1)`, () => {
        expect(round(m.faintOnCanvas), 'faint on canvas').toBeGreaterThanOrEqual(largeTarget);
      });

      it('separates the accent from what it sits on (needs 3:1)', () => {
        expect(round(m.accentOnCanvas), 'accent on canvas').toBeGreaterThanOrEqual(LARGE);
        expect(round(m.accentOnSurface), 'accent on surface').toBeGreaterThanOrEqual(LARGE);
      });

      /** The primary action carries white 13px text, and its gradient is
       *  derived precisely so this can never depend on the accent chosen. */
      it('keeps white text on the primary action readable', () => {
        expect(round(m.whiteOnBrand), 'white on brand').toBeGreaterThanOrEqual(4.5);
      });
    });
  }
});

describe('layer relationships', () => {
  /**
   * Each layer must be tellable from the one below it.
   *
   * Deliberately not "canvas is always darkest". The light themes step *up* to
   * a white surface and then *down* to a slightly sunk `raised` for rows and
   * inputs, which is correct and is what the shipped Pure Light does — a test
   * that demanded one direction would have failed the design rather than a bug.
   * What actually matters is that no two adjacent layers collapse into each
   * other, because then a card stops reading as a card.
   */
  it('keeps every layer distinguishable from the one below it', () => {
    const luminance = (hex: string) => {
      const { r, g, b } = parseHex(hex);
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };

    for (const theme of THEMES) {
      const { colour } = resolveTheme(theme);
      const at = (key: string): string => colour[key] ?? '';

      const steps: [string, string, string][] = [
        ['canvas', 'surface', at('surface')],
        ['surface', 'raised', at('raised')],
        ['raised', 'overlay', at('overlay')],
      ];

      let previous = at('canvas');
      for (const [from, to, value] of steps) {
        const delta = Math.abs(luminance(value) - luminance(previous));
        expect(delta, `${theme.id}: ${from} and ${to} are the same shade`).toBeGreaterThanOrEqual(
          2,
        );
        previous = value;
      }
    }
  });

  it('keeps ink brighter-contrasting than muted on the canvas', () => {
    for (const theme of THEMES) {
      const { colour } = resolveTheme(theme);
      const at = (key: string): string => colour[key] ?? '';

      expect(
        contrastRounded(at('ink'), at('canvas')),
        `${theme.id}: ink must out-contrast muted`,
      ).toBeGreaterThanOrEqual(contrastRounded(at('muted'), at('canvas')));
    }
  });
});
