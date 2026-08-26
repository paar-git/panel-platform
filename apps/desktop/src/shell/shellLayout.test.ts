/**
 * Reading and writing where the shell was.
 *
 * The loader's contract is that nothing it reads can stop the window opening.
 * A file written by an older build, a truncated one, a hand-edited one with
 * the wrong types in it — each has to produce a usable layout rather than an
 * exception, and each is checked here.
 */
import { describe, expect, it } from 'vitest';

import {
  defaultShellLayout,
  loadShellLayout,
  saveShellLayout,
  SECTIONS,
  type LayoutStorage,
} from './shellLayout';

/** A storage that only holds what this test puts in it. */
function storage(initial?: string): LayoutStorage & { written: string | null } {
  let value = initial ?? null;
  return {
    getItem: () => value,
    setItem: (_key, next) => {
      value = next;
    },
    get written() {
      return value;
    },
  };
}

describe('loadShellLayout', () => {
  it('uses the defaults when nothing has been stored', () => {
    expect(loadShellLayout(storage())).toEqual(defaultShellLayout);
  });

  it('uses the defaults when there is no storage at all', () => {
    // Server-side rendering and the tests both reach this branch.
    expect(loadShellLayout(undefined)).toEqual(defaultShellLayout);
  });

  it('survives a file that is not JSON', () => {
    expect(loadShellLayout(storage('{ not json'))).toEqual(defaultShellLayout);
  });

  it('survives JSON that is not an object', () => {
    for (const raw of ['null', '42', '"a string"', '[]']) {
      expect(loadShellLayout(storage(raw)), raw).toEqual(
        raw === '[]' ? { ...defaultShellLayout } : defaultShellLayout,
      );
    }
  });

  it('reads a layout it wrote itself', () => {
    const written = storage();
    saveShellLayout(written, {
      section: 'discord',
      sidebarCollapsed: true,
      projectId: 'prj_1',
    });

    expect(loadShellLayout(written)).toEqual({
      section: 'discord',
      sidebarCollapsed: true,
      projectId: 'prj_1',
    });
  });

  it('keeps the fields it recognises and defaults the rest', () => {
    // Half-keeping is the point: a layout written by an older build should
    // not cost the user every remembered field.
    const half = loadShellLayout(storage(JSON.stringify({ section: 'settings' })));

    expect(half.section).toBe('settings');
    expect(half.sidebarCollapsed).toBe(defaultShellLayout.sidebarCollapsed);
    expect(half.projectId).toBeNull();
  });

  it('refuses a section this build does not have', () => {
    // The stored value is a string from disk, so it is not enough that the
    // type says it cannot be `resources` — a `v1` file is full of them.
    const stored = loadShellLayout(storage(JSON.stringify({ section: 'resources' })));
    expect(stored.section).toBe(defaultShellLayout.section);
  });

  it('accepts every section this build offers', () => {
    for (const section of SECTIONS) {
      expect(loadShellLayout(storage(JSON.stringify({ section }))).section).toBe(section);
    }
  });

  it('refuses a project id that is not a string', () => {
    expect(loadShellLayout(storage(JSON.stringify({ projectId: 7 }))).projectId).toBeNull();
  });
});

describe('saveShellLayout', () => {
  it('writes the layout it was given', () => {
    const written = storage();
    saveShellLayout(written, defaultShellLayout);
    expect(JSON.parse(written.written ?? 'null')).toEqual(defaultShellLayout);
  });

  it('ignores a storage that refuses to write', () => {
    // Private browsing and a full quota both throw here. Losing a remembered
    // section is not worth an error in front of anyone.
    const refusing: LayoutStorage = {
      getItem: () => null,
      setItem: () => {
        throw new Error('quota exceeded');
      },
    };

    expect(() => saveShellLayout(refusing, defaultShellLayout)).not.toThrow();
  });
});
