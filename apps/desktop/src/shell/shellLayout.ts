/**
 * Where the application is, and whether its rail is collapsed.
 *
 * Deliberately not shared with `workspace/layout.ts`: the editor's layout and
 * the shell's are two different arrangements that happen to have similar
 * fields, and folding them together would mean a change to one silently
 * rearranging the other.
 *
 * Persisted for the reason that file gives — a shell that forgets where you
 * were every launch is one nobody settles into.
 *
 * The rail no longer has a width to remember. It is a fixed column of places
 * rather than a resizable panel of tools, so the only thing worth storing
 * about it is whether it is collapsed. The key is `v2` because a stored `v1`
 * describes a layout this shell cannot render.
 */

/** Which section the rail is on. */
export type SectionId = 'overview' | 'projects' | 'activity' | 'discord' | 'settings';

export const SECTIONS: SectionId[] = ['overview', 'projects', 'activity', 'discord', 'settings'];

export interface ShellLayout {
  section: SectionId;
  sidebarCollapsed: boolean;
  /** Which project the shell is pointed at, or null for none. */
  projectId: string | null;
}

export const defaultShellLayout: ShellLayout = {
  section: 'overview',
  sidebarCollapsed: false,
  projectId: null,
};

const STORAGE_KEY = 'shell.layout.v2';

/** The subset of `Storage` this module uses. Narrow enough to fake in a test. */
export interface LayoutStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/**
 * Read the stored layout, falling back field by field.
 *
 * Field by field rather than a shape check: a layout written by an older
 * version is worth half-keeping, and a corrupt one must never stop the window
 * from opening.
 */
export function loadShellLayout(storage: LayoutStorage | undefined): ShellLayout {
  const raw = storage?.getItem(STORAGE_KEY);
  if (!raw) return defaultShellLayout;

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return defaultShellLayout;
  }
  if (parsed === null || typeof parsed !== 'object') return defaultShellLayout;

  const stored = parsed as Partial<Record<keyof ShellLayout, unknown>>;
  return {
    section:
      typeof stored.section === 'string' && (SECTIONS as string[]).includes(stored.section)
        ? (stored.section as SectionId)
        : defaultShellLayout.section,
    sidebarCollapsed:
      typeof stored.sidebarCollapsed === 'boolean'
        ? stored.sidebarCollapsed
        : defaultShellLayout.sidebarCollapsed,
    // Not validated against the project list here: this module cannot know
    // what exists, and the shell drops an id that no longer resolves.
    projectId: typeof stored.projectId === 'string' ? stored.projectId : null,
  };
}

/** Write the layout. A storage that refuses (private mode, quota) is ignored. */
export function saveShellLayout(storage: LayoutStorage | undefined, layout: ShellLayout): void {
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify(layout));
  } catch {
    // Losing a remembered section is not worth an error in front of anyone.
  }
}
