import { describe, expect, it } from 'vitest';

import { THEMES } from './index';
import { filterThemes, groupByCategory, matchesText } from './search';

const find = (id: string) => {
  const theme = THEMES.find((candidate) => candidate.id === id);
  if (!theme) throw new Error(`no such theme: ${id}`);
  return theme;
};

describe('matching text', () => {
  it('matches everything on an empty term', () => {
    for (const theme of THEMES) expect(matchesText(theme, '')).toBe(true);
    expect(matchesText(find('dracula'), '   ')).toBe(true);
  });

  it('matches a name regardless of case', () => {
    expect(matchesText(find('vaporwave'), 'VAPOR')).toBe(true);
  });

  it('matches words from the description', () => {
    // Forest's detail talks about canopy and moss; its name says none of that.
    expect(matchesText(find('forest'), 'moss')).toBe(true);
  });

  it('matches the category a theme is in', () => {
    expect(matchesText(find('system-95'), 'retro')).toBe(true);
  });

  /** Every term has to hit something, so a second word narrows rather than
   *  widens — otherwise "dark green" returns every dark theme. */
  it('requires every term to match', () => {
    expect(matchesText(find('matrix-rain'), 'matrix green')).toBe(true);
    expect(matchesText(find('matrix-rain'), 'matrix parchment')).toBe(false);
  });

  it('ignores punctuation and hyphens', () => {
    expect(matchesText(find('8-bit-console'), '8 bit')).toBe(true);
    expect(matchesText(find('8-bit-console'), '8-bit')).toBe(true);
  });

  it('finds a theme by its id', () => {
    expect(matchesText(find('tokyo-night'), 'tokyo-night')).toBe(true);
  });
});

describe('filtering', () => {
  it('returns everything for an empty query', () => {
    expect(filterThemes(THEMES, {})).toHaveLength(THEMES.length);
  });

  it('narrows to one category', () => {
    const result = filterThemes(THEMES, { category: 'developer' });
    // Eleven: developer holds `panel-warm`, the application's own palette.
    expect(result).toHaveLength(11);
    for (const theme of result) expect(theme.category).toBe('developer');
  });

  it('applies text and category together', () => {
    const result = filterThemes(THEMES, { category: 'nature', text: 'blue' });
    for (const theme of result) expect(theme.category).toBe('nature');
    expect(result.length).toBeGreaterThan(0);
    expect(result.length).toBeLessThan(10);
  });

  it('returns nothing rather than everything when nothing matches', () => {
    expect(filterThemes(THEMES, { text: 'zzzznotathing' })).toEqual([]);
  });

  it('keeps the catalogue order', () => {
    const result = filterThemes(THEMES, { text: 'a' });
    const positions = result.map((theme) => THEMES.indexOf(theme));
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
  });
});

describe('grouping', () => {
  it('groups the whole catalogue into its eight categories', () => {
    expect(groupByCategory(THEMES)).toHaveLength(8);
  });

  it('drops groups with nothing in them', () => {
    const groups = groupByCategory(filterThemes(THEMES, { category: 'gaming' }));
    expect(groups).toHaveLength(1);
    expect(groups[0]?.category).toBe('gaming');
  });

  it('loses no theme along the way', () => {
    const total = groupByCategory(THEMES).reduce((sum, group) => sum + group.themes.length, 0);
    expect(total).toBe(THEMES.length);
  });
});
