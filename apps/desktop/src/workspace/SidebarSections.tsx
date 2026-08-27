/**
 * The sidebars that are not the Explorer.
 *
 * Search is real: it calls the core's `search_project_files`, which walks the
 * project and matches on the file *name*. Searching inside file contents is not
 * something the core does, and the panel says which of the two it is rather
 * than leaving the user to wonder why a string they can see is not found.
 *
 * Source Control, Run and Extensions exist because the activity bar is a fixed
 * set of sections. Each one states plainly what this application does instead,
 * which is more use than an icon that opens an empty box.
 */
import { useEffect, useState } from 'react';

import type { FileEntry, ProjectSummary, SystemStatus } from '../api';
import Icon from './Icon';
import { fileIconColor } from './fileIcons';
import { parentOf, tabLabel } from './tabs';

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-9 shrink-0 items-center px-5 text-[11px] tracking-wide text-vs-dim uppercase">
      {children}
    </div>
  );
}

function SectionNote({ children }: { children: React.ReactNode }) {
  return <p className="px-5 py-2 text-[13px] leading-relaxed text-vs-dim">{children}</p>;
}

// ------------------------------------------------------------------- search

export function SearchPanel({
  query,
  onQueryChange,
  results,
  searching,
  failure,
  onOpen,
}: {
  query: string;
  onQueryChange: (query: string) => void;
  results: FileEntry[];
  searching: boolean;
  failure: string | null;
  onOpen: (entry: FileEntry) => void;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SectionTitle>Search</SectionTitle>

      <div className="px-3 pb-2">
        <input
          value={query}
          autoFocus
          onChange={(event) => onQueryChange(event.target.value)}
          placeholder="Search files by name"
          aria-label="Search files by name"
          className="h-[26px] w-full border border-vs-border bg-vs-editor px-2 text-[13px] text-vs-text outline-none select-text focus:border-accent"
        />
        <p className="mt-1.5 text-[11px] text-vs-dim">
          Matches file and folder names. The core does not index file contents.
        </p>
      </div>

      {failure && <p className="px-3 pb-2 text-[12px] text-red-400">{failure}</p>}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {query.trim().length === 0 ? null : searching ? (
          <SectionNote>Searching…</SectionNote>
        ) : results.length === 0 ? (
          <SectionNote>No file name matches “{query.trim()}”.</SectionNote>
        ) : (
          <ul>
            {results.map((entry) => (
              <li key={entry.path}>
                <button
                  type="button"
                  onClick={() => onOpen(entry)}
                  title={entry.path}
                  className="flex h-[22px] w-full items-center gap-1.5 px-3 text-left hover:bg-white/5"
                >
                  <span
                    className="shrink-0"
                    style={{
                      color: entry.kind === 'directory' ? '#8aa2c8' : fileIconColor(entry.name),
                    }}
                  >
                    <Icon name={entry.kind === 'directory' ? 'folder' : 'file'} size={15} />
                  </span>
                  <span className="truncate text-[13px] text-vs-text">{entry.name}</span>
                  <span className="min-w-0 truncate text-[12px] text-vs-dim">
                    {parentOf(entry.path)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

/**
 * Ask the core for name matches, a moment after typing stops.
 *
 * Debounced because the search walks the project directory: firing it per
 * keystroke would queue one walk per character on a large tree.
 */
export function useFileSearch(
  projectId: string,
  query: string,
  search: (projectId: string, query: string) => Promise<FileEntry[]>,
  onError: (error: unknown) => string,
): { results: FileEntry[]; searching: boolean; failure: string | null } {
  const [results, setResults] = useState<FileEntry[]>([]);
  const [searching, setSearching] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    const trimmed = query.trim();
    if (trimmed.length === 0) {
      setResults([]);
      setSearching(false);
      setFailure(null);
      return;
    }

    let cancelled = false;
    setSearching(true);
    const timer = setTimeout(() => {
      search(projectId, trimmed)
        .then((entries) => {
          if (cancelled) return;
          setResults(entries);
          setFailure(null);
        })
        .catch((error: unknown) => {
          if (!cancelled) setFailure(onError(error));
        })
        .finally(() => {
          if (!cancelled) setSearching(false);
        });
    }, 200);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [projectId, query, search, onError]);

  return { results, searching, failure };
}

// ----------------------------------------------------------- source control

export function SourceControlPanel({ projectRoot }: { projectRoot: string | null }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SectionTitle>Source Control</SectionTitle>
      <SectionNote>
        This application does not manage a repository for a project. A project created from a git
        remote was cloned once, and its folder on this machine is an ordinary folder from then on.
      </SectionNote>
      {projectRoot && (
        <p className="px-5 text-[12px] break-all text-vs-dim select-text">
          Use your own git client in <span className="font-mono text-vs-text">{projectRoot}</span>.
        </p>
      )}
    </div>
  );
}

// ------------------------------------------------------------ run and debug

export function RunPanel({
  project,
  onOpenTerminal,
}: {
  project: ProjectSummary;
  onOpenTerminal: () => void;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SectionTitle>Run and Debug</SectionTitle>

      <div className="px-3">
        <button
          type="button"
          onClick={onOpenTerminal}
          className="flex w-full items-center justify-center gap-2 rounded-[3px] bg-accent px-3 py-1.5 text-[13px] font-medium text-white hover:brightness-110"
        >
          <Icon name="play" size={14} />
          Open the run controls
        </button>
      </div>

      <dl className="mt-3 px-5 text-[12px]">
        <Fact label="Status" value={project.status.toLowerCase()} />
        <Fact label="Wanted" value={project.desiredState.toLowerCase()} />
        <Fact label="Type" value={project.projectType.toLowerCase()} />
      </dl>

      <SectionNote>
        There is no debugger. A project runs as its own process or container; attaching to it is not
        something this application does.
      </SectionNote>
    </div>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline gap-2 py-0.5">
      <dt className="w-20 shrink-0 text-vs-dim">{label}</dt>
      <dd className="min-w-0 truncate text-vs-text">{value}</dd>
    </div>
  );
}

// -------------------------------------------------------------- extensions

export function ExtensionsPanel() {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SectionTitle>Extensions</SectionTitle>
      <SectionNote>
        The editor here is Monaco with the highlighting it ships with. There is no extension host,
        so there is nothing to install — and nothing running in this window that you did not.
      </SectionNote>
    </div>
  );
}

// ------------------------------------------------------------------ account

export function AccountPanel({
  status,
  openFiles,
}: {
  status: SystemStatus | null;
  openFiles: string[];
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SectionTitle>Account</SectionTitle>
      <SectionNote>
        There is no account. Everything this application does happens on this machine, under the
        user you are already signed in as.
      </SectionNote>

      {status && (
        <dl className="px-5 text-[12px]">
          <Fact label="Version" value={status.appVersion} />
          <Fact label="Schema" value={`v${status.schemaVersion}`} />
        </dl>
      )}

      {openFiles.length > 0 && (
        <>
          <div className="mt-3 px-5 text-[11px] tracking-wide text-vs-dim uppercase">
            Open editors
          </div>
          <ul className="px-5 py-1 text-[12px]">
            {openFiles.map((path) => (
              <li key={path} className="truncate text-vs-text" title={path}>
                {tabLabel(path)}
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
