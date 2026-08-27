import { describe, expect, it } from 'vitest';

import type { ProjectSummary } from '../api';
import {
  applyListOptions,
  defaultListOptions,
  isFiltered,
  runtimeLabel,
  runtimesIn,
  type ListOptions,
} from './projectList';

function project(
  displayName: string,
  status: string,
  projectType = 'NODEJS',
  description = '',
): ProjectSummary {
  return {
    id: displayName,
    slug: displayName.toLowerCase().replace(/\s+/g, '-'),
    displayName,
    description,
    projectType,
    status,
    desiredState: 'RUNNING',
    color: null,
  };
}

const projects = [
  project('Zebra API', 'RUNNING', 'GO'),
  project('alpha bot', 'FAILED', 'PYTHON', 'a discord bot'),
  project('Midway', 'STOPPED', 'NODEJS'),
  project('Nimbus', 'STARTING', 'NODEJS'),
];

function options(patch: Partial<ListOptions> = {}): ListOptions {
  return { ...defaultListOptions, ...patch };
}

describe('searching', () => {
  it('matches the name regardless of case', () => {
    expect(
      applyListOptions(projects, options({ query: 'ZEBRA' })).map((p) => p.displayName),
    ).toEqual(['Zebra API']);
  });

  it('matches the slug and the description too', () => {
    expect(applyListOptions(projects, options({ query: 'alpha-bot' })).map((p) => p.id)).toEqual([
      'alpha bot',
    ]);
    expect(applyListOptions(projects, options({ query: 'discord' })).map((p) => p.id)).toEqual([
      'alpha bot',
    ]);
  });

  it('ignores surrounding whitespace', () => {
    expect(applyListOptions(projects, options({ query: '  midway  ' }))).toHaveLength(1);
  });

  it('returns everything for an empty query', () => {
    expect(applyListOptions(projects, options())).toHaveLength(4);
  });
});

describe('status filters', () => {
  it('picks out running, failed and stopped', () => {
    expect(applyListOptions(projects, options({ status: 'running' })).map((p) => p.id)).toEqual([
      'Zebra API',
    ]);
    expect(applyListOptions(projects, options({ status: 'failed' })).map((p) => p.id)).toEqual([
      'alpha bot',
    ]);
  });

  it('counts a transitioning project as stopped rather than dropping it', () => {
    // A starting project is not running yet and has not failed. Leaving it out
    // of every filter is how a project disappears from the list entirely.
    const stopped = applyListOptions(projects, options({ status: 'stopped' })).map((p) => p.id);
    expect(stopped).toContain('Midway');
    expect(stopped).toContain('Nimbus');
  });
});

describe('runtime filter', () => {
  it('narrows to one runtime', () => {
    expect(applyListOptions(projects, options({ runtime: 'NODEJS' })).map((p) => p.id)).toEqual([
      'Midway',
      'Nimbus',
    ]);
  });

  it('lists the runtimes actually present, sorted and deduplicated', () => {
    expect(runtimesIn(projects)).toEqual(['GO', 'NODEJS', 'PYTHON']);
  });
});

describe('sorting', () => {
  it('sorts by name without letting case decide', () => {
    expect(applyListOptions(projects, options({ sort: 'name' })).map((p) => p.displayName)).toEqual(
      ['alpha bot', 'Midway', 'Nimbus', 'Zebra API'],
    );
  });

  it('puts what is broken first when sorting by status', () => {
    // The reason to sort by status is to find the problem.
    expect(applyListOptions(projects, options({ sort: 'status' })).map((p) => p.id)).toEqual([
      'alpha bot',
      'Nimbus',
      'Zebra API',
      'Midway',
    ]);
  });

  it('groups by runtime, then by name', () => {
    expect(applyListOptions(projects, options({ sort: 'runtime' })).map((p) => p.id)).toEqual([
      'Zebra API',
      'Midway',
      'Nimbus',
      'alpha bot',
    ]);
  });

  it('never reorders the array it was given', () => {
    const before = [...projects];
    applyListOptions(projects, options({ sort: 'status' }));
    expect(projects).toEqual(before);
  });
});

describe('combining', () => {
  it('filters and sorts together', () => {
    const result = applyListOptions(
      projects,
      options({ runtime: 'NODEJS', sort: 'name', query: 'i' }),
    );
    expect(result.map((p) => p.id)).toEqual(['Midway', 'Nimbus']);
  });
});

describe('knowing a filter is on', () => {
  it('is false for the defaults and true for anything set', () => {
    expect(isFiltered(defaultListOptions)).toBe(false);
    expect(isFiltered(options({ query: 'x' }))).toBe(true);
    expect(isFiltered(options({ status: 'failed' }))).toBe(true);
    expect(isFiltered(options({ runtime: 'GO' }))).toBe(true);
    // Sorting is not filtering: it hides nothing.
    expect(isFiltered(options({ sort: 'status' }))).toBe(false);
  });
});

describe('runtime names', () => {
  it('spells out the ones it knows', () => {
    expect(runtimeLabel('NODEJS')).toBe('Node.js');
    expect(runtimeLabel('DOTNET')).toBe('.NET');
  });

  it('renders an unknown runtime rather than hiding it', () => {
    expect(runtimeLabel('ELIXIR')).toBe('Elixir');
    expect(runtimeLabel('SOME_NEW_THING')).toBe('Some new thing');
  });
});
