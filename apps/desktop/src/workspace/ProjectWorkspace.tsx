import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import * as monaco from 'monaco-editor';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  appendProjectFileUpload,
  appSettings,
  beginProjectFileUpload,
  cancelProjectFileImport,
  cancelProjectFileUpload,
  copyProjectFile,
  createProjectFile,
  deleteProjectFile,
  errorMessage,
  finishProjectFileUpload,
  importProjectFiles,
  inspectImportPaths,
  killProject,
  planImportDestinations,
  listProjectFiles,
  moveProjectFile,
  onFileImportProgress,
  projectRootPath,
  readProjectFile,
  renameProjectFile,
  restartProject,
  revealProjectPath,
  searchProjectFiles,
  startProject,
  stopProject,
  writeProjectFile,
  type AppSettings,
  type FileEntry,
  type ImportCandidate,
  type PlannedDestination,
  type ProjectSummary,
  type SystemStatus,
} from '../api';
import { updateStore, useUpdate } from '../useUpdate';
import ActivityBar from './ActivityBar';
import BottomPanel from './BottomPanel';
import CodeEditor, { type CursorPosition, type RevealRequest } from './CodeEditor';
import CommandPalette, { type PaletteMode } from './CommandPalette';
import type { Command } from './commands';
import { conflictingPaths, resolveConflicts, type ConflictChoice } from './conflicts';
import ContextMenu, { type MenuEntry, type MenuPosition } from './ContextMenu';
import { ConfirmDialog, ConflictDialog, ImportPreviewDialog } from './Dialogs';
import BatchConflictDialog from './ConflictDialog';
import {
  allConflicts,
  analyse,
  planIsStale,
  resolvePlan,
  type Conflict,
  type Decisions,
  type ItemKind,
  type Operation,
  type PlannedItem,
  type ResolvedItem,
  type ResolvedPlan,
} from './conflictResolution';
import {
  noGrouping,
  pickableDirectories,
  planRelocation,
  preserveDecisions,
  type PlanGrouping,
  type RelocationPlan,
  type RelocationRequest,
} from './relocation';
import { groupingFor, relocateGroups } from './organiserRelocation';
import { runReplacementTransaction } from './replacementTransaction';
import { describePlan, explainDetection, planImport, type ImportPlan } from './importPlan';
import ImportOrganiser from './ImportOrganiser';
import ImportProgressDialog from './ImportProgressDialog';
import {
  addError,
  advanceRollback,
  applyBatchProgress,
  completeBatch,
  enterCommit,
  enterPhase,
  finish,
  newOperation,
  requestCancellation,
  shouldRender,
  startRollback,
  type BatchProgress,
  type ImportOperationProgress,
  type OperationBatch,
} from './importOperation';
import { groupsFrom, planFrom, summarise, type ImportGroup } from './importGroups';
import {
  cleanDropPath,
  collectDroppedItems,
  directoryPathsForDrop,
  duplicateDroppedFilePath,
  isInsideDropZone,
  shouldImportBrowserDrop,
  type DropPoint,
  type DroppedItems,
} from './dropImport';
import EditorTabs from './EditorTabs';
import EmptyEditorState, { type WelcomeAction } from './EmptyEditorState';
import ExplorerPanel from './ExplorerPanel';
import type { InlineEdit, TreeState } from './FileTree';
import { languageName } from './fileIcons';
import Icon from './Icon';
import { baseName } from '../lib/format';
import {
  clampPanelHeight,
  clampSidebarWidth,
  loadLayout,
  saveLayout,
  type ActivityView,
  type Layout,
  type PanelTab,
} from './layout';
import { appendLine, linesForChannel, type OutputChannel, type OutputLine } from './output';
import { LogsPanel, OutputPanel, ProblemsPanel, TerminalPanel, type RunAction } from './panels';
import { countProblems, useProblems, type Problem } from './problems';
import ResizeHandle from './ResizeHandle';
import { isTypingTarget } from './shortcuts';
import {
  clearSelection,
  dragPaths as dragPathsFor,
  emptySelection,
  focusEdge,
  selectFromPointer,
  isEditableTarget,
  moveFocus,
  pruneSelection,
  renameInSelection,
  selectAll,
  selectOnly,
  selectPaths,
  selectionForContextMenu,
  type Selection,
  type VisibleEntry,
} from './selection';
import { describePaste, pasteDestination, planMove, planPaste, type Clipboard } from './clipboard';
import {
  AccountPanel,
  ExtensionsPanel,
  RunPanel,
  SearchPanel,
  SourceControlPanel,
  useFileSearch,
} from './SidebarSections';
import { isDeclined, useToolchainGate } from '../components/useToolchainGate';
import StatusBar from './StatusBar';
import {
  activeBuffer,
  bufferFor,
  childPath,
  closeAll,
  closeFile,
  closeOthers,
  closeToRight,
  edit,
  emptyEditor,
  forgetDeleted,
  isDirty,
  markSaved,
  openFile,
  parentOf,
  renamePath,
  tabLabel,
  toggleExpanded,
  type EditorState,
} from './tabs';
import TitleBar from './TitleBar';
import type { Menu } from './MenuBar';
import {
  createUploadId,
  displayDirectory,
  nativeImportLabel,
  UPLOAD_CANCELLED,
  UPLOAD_CHUNK_BYTES,
  type BrowserUploadItem,
  type NativeImportItem,
  type UploadItem,
  type UploadPatch,
} from './uploads';

/**
 * One project, as a workspace.
 *
 * This is the whole editor: the menu bar, the activity bar, the explorer, the
 * tabs, Monaco, the panel and the status bar. It replaced two screens — a file
 * browser and a separate console — because they were two halves of the same
 * job and switching between them lost the editor's state.
 *
 * The file it is not: a component library. Everything visual is a component in
 * this folder, and everything with a rule in it that can be got wrong is a
 * tested pure function in `tabs.ts`, `layout.ts`, `conflicts.ts` or
 * `commands.ts`. What is left here is the wiring — the API calls, and the state
 * that several panes have to agree on.
 *
 * Every path sent from here is *relative to the project root*. The core takes a
 * project id and a relative string and builds the real path itself, so this
 * view has no way to name a file outside the project even if it tried.
 */
export default function ProjectWorkspace({
  project,
  status,
  onRefreshProjects,
  onLeave,
  onOpenSettings,
  onOpenUpdates,
}: {
  project: ProjectSummary;
  status: SystemStatus | null;
  onRefreshProjects: () => Promise<void>;
  onLeave: () => void;
  onOpenSettings: () => void;
  /** Opens the update manager, which is rendered by the shell above this one. */
  onOpenUpdates: () => void;
}) {
  // ------------------------------------------------------------------ state
  const [listings, setListings] = useState<Record<string, FileEntry[]>>({});
  const [expanded, setExpanded] = useState<string[]>([]);
  const [editor, setEditor] = useState<EditorState>(emptyEditor);
  const [selection, setSelection] = useState<Selection>(emptySelection);
  const [clipboard, setClipboard] = useState<Clipboard | null>(null);
  const [targetDirectory, setTargetDirectory] = useState('');
  const [editing, setEditing] = useState<InlineEdit | null>(null);
  const [saving, setSaving] = useState(false);
  const [cursor, setCursor] = useState<CursorPosition | null>(null);
  const [reveal, setReveal] = useState<RevealRequest | null>(null);

  const [layout, setLayout] = useState<Layout>(() =>
    loadLayout(typeof window === 'undefined' ? undefined : window.localStorage),
  );
  const [palette, setPalette] = useState<PaletteMode | null>(null);
  const [menu, setMenu] = useState<{ entries: MenuEntry[]; position: MenuPosition } | null>(null);
  const [confirm, setConfirm] = useState<{
    title: string;
    detail?: string;
    confirmLabel: string;
    danger?: boolean;
    onConfirm: () => void;
  } | null>(null);

  const [uploads, setUploads] = useState<UploadItem[]>([]);
  const [draggingOver, setDraggingOver] = useState(false);
  const [conflict, setConflict] = useState<{
    paths: string[];
    directory: string;
    resolve: (choice: ConflictChoice | null) => void;
  } | null>(null);
  /** A drop waiting for the user to confirm how it should be laid out. */
  const [importPreview, setImportPreview] = useState<{
    plan: ImportPlan;
    directory: string;
  } | null>(null);
  /** A collision the user has to settle before anything is written. */
  const [conflictReview, setConflictReview] = useState<{
    conflicts: Conflict[];
    operation: Operation;
    existing: string[];
    items: PlannedItem[];
    existingKinds: Map<string, ItemKind>;
    grouping: PlanGrouping;
    directories: string[];
    initialDecisions?: Decisions;
    notice: string | null;
    /**
     * Bumped every time the batch is re-analysed, so the dialog remounts with
     * the new conflicts instead of holding decisions about the old ones.
     */
    revision: number;
    resolve: (outcome: ConflictOutcome) => void;
  } | null>(null);
  /** The organised import that is running, if any. */
  const [operationProgress, setOperationProgress] = useState<ImportOperationProgress | null>(null);
  /** A drop complex enough to need the full organiser. */
  const [organising, setOrganising] = useState<{
    groups: ImportGroup[];
    candidates: ImportCandidate[];
    directory: string;
    existing: string[];
  } | null>(null);

  const [output, setOutput] = useState<OutputLine[]>([]);
  const [outputChannel, setOutputChannel] = useState<OutputChannel | 'all'>('all');
  const [busyAction, setBusyAction] = useState<RunAction | null>(null);
  const { gate, guard } = useToolchainGate();
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [projectRoot, setProjectRoot] = useState<string | null>(null);

  const [searchQuery, setSearchQuery] = useState('');
  const [quickOpenTerm, setQuickOpenTerm] = useState('');

  /** Where Back and Forward go: the files opened, in order. */
  const [history, setHistory] = useState<{ paths: string[]; index: number }>({
    paths: [],
    index: -1,
  });

  const cancelledUploads = useRef(new Set<string>());
  const dragDepth = useRef(0);
  const outputId = useRef(0);
  /**
   * Whether Tauri's OS-level drag/drop listener is live. While it is, it owns
   * every drop and the HTML5 handlers below do nothing — see
   * `shouldImportBrowserDrop`.
   */
  const nativeListenerReady = useRef(false);
  const explorer = useRef<HTMLDivElement | null>(null);

  /**
   * The live values the keyboard and clipboard handlers need.
   *
   * Held in refs so those handlers can be bound once instead of re-binding on
   * every keystroke that changes a listing — and so a callback created during
   * one render never acts on a selection from that render.
   */
  const selectionRef = useRef(selection);
  selectionRef.current = selection;
  const listingsRef = useRef(listings);
  listingsRef.current = listings;
  const clipboardRef = useRef(clipboard);
  clipboardRef.current = clipboard;
  const targetDirectoryRef = useRef(targetDirectory);
  targetDirectoryRef.current = targetDirectory;
  const expandedRef = useRef(expanded);
  expandedRef.current = expanded;

  /**
   * The operation the progress dialog is showing.
   *
   * Events from an operation the user has already moved past must not repaint
   * a newer one, so every publish checks this first.
   */
  const operationRef = useRef<string | null>(null);
  /** When the progress dialog was last redrawn, for throttling. */
  const lastProgressRender = useRef(0);
  /** Operations the user has asked to stop. */
  const cancelledOperations = useRef(new Set<string>());
  /** The batches in flight, so their core events can be folded in. */
  const activeImports = useRef(
    new Map<
      string,
      {
        operationId: string;
        batches: OperationBatch[];
        finished: { entries: number; bytes: number };
        apply: (event: BatchProgress) => void;
      }
    >(),
  );

  /**
   * Every row the tree is drawing, in the order it draws them.
   *
   * This is what a Shift-range measures across and what Ctrl+A selects, so it
   * has to be built the same way the tree walks itself: a folder contributes
   * its children only while it is expanded.
   */
  const visibleEntries = useMemo<VisibleEntry[]>(() => {
    const rows: VisibleEntry[] = [];
    const walk = (directory: string) => {
      for (const entry of listings[directory] ?? []) {
        rows.push({ path: entry.path, isDirectory: entry.kind === 'directory' });
        if (entry.kind === 'directory' && expanded.includes(entry.path)) walk(entry.path);
      }
    };
    walk('');
    return rows;
  }, [listings, expanded]);

  const visibleRef = useRef(visibleEntries);
  visibleRef.current = visibleEntries;

  // A refresh, a delete or an import can take rows away underneath a
  // selection. Keeping paths that no longer exist makes the count lie and the
  // next Delete ask about files that are already gone.
  useEffect(() => {
    setSelection((current) =>
      current.selected.length === 0
        ? current
        : pruneSelection(
            current,
            visibleEntries.map((entry) => entry.path),
          ),
    );
  }, [visibleEntries]);

  const problems = useProblems();
  const { errors, warnings } = countProblems(problems);
  const update = useUpdate();

  const buffer = activeBuffer(editor);
  const dirtyCount = editor.buffers.filter(isDirty).length;

  // ----------------------------------------------------------------- output
  const say = useCallback(
    (channel: OutputChannel, text: string, level: OutputLine['level'] = 'info') => {
      outputId.current += 1;
      setOutput((current) =>
        appendLine(current, { id: outputId.current, at: new Date(), channel, level, text }),
      );
    },
    [],
  );

  const report = useCallback(
    (channel: OutputChannel, error: unknown) => {
      say(channel, errorMessage(error), 'error');
    },
    [say],
  );

  // ------------------------------------------------------------- the layout
  const patchLayout = useCallback((patch: Partial<Layout>) => {
    setLayout((current) => {
      const next = { ...current, ...patch };
      saveLayout(typeof window === 'undefined' ? undefined : window.localStorage, next);
      return next;
    });
  }, []);

  const showPanel = useCallback(
    (tab: PanelTab) => patchLayout({ panelVisible: true, panelTab: tab }),
    [patchLayout],
  );

  const showSidebar = useCallback(
    (view: ActivityView) => patchLayout({ sidebarVisible: true, activityView: view }),
    [patchLayout],
  );

  // The window shrinking below a stored size has to move the panels, or the
  // editor is squeezed out of existence by a layout from a bigger monitor.
  useEffect(() => {
    function onResize() {
      setLayout((current) => ({
        ...current,
        sidebarWidth: clampSidebarWidth(current.sidebarWidth, window.innerWidth),
        panelHeight: clampPanelHeight(current.panelHeight, window.innerHeight),
      }));
    }
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  // ----------------------------------------------------------- loading files
  const loadDirectory = useCallback(
    async (path: string) => {
      try {
        const listing = await listProjectFiles(project.id, path);
        setListings((current) => ({ ...current, [path]: listing.entries }));
        if (listing.truncated) {
          say(
            'Files',
            `${displayDirectory(path)} has more entries than the core will return at once; some are not shown.`,
            'warn',
          );
        }
      } catch (error) {
        report('Files', error);
      }
    },
    [project.id, report, say],
  );

  useEffect(() => {
    void loadDirectory('');
  }, [loadDirectory]);

  useEffect(() => {
    projectRootPath(project.id)
      .then(setProjectRoot)
      .catch(() => setProjectRoot(null));
    appSettings()
      .then(setSettings)
      .catch(() => setSettings(null));
  }, [project.id]);

  /** Reload one folder and every folder above it. */
  const refreshBranch = useCallback(
    async (path: string) => {
      const directories = new Set<string>(['']);
      let directory = parentOf(path);
      directories.add(directory);
      while (directory) {
        directory = parentOf(directory);
        directories.add(directory);
      }
      await Promise.all([...directories].map((item) => loadDirectory(item)));
    },
    [loadDirectory],
  );

  /** Which paths are known to exist under a folder, for the conflict check. */
  const pathsIn = useCallback(
    async (directory: string): Promise<string[]> => {
      try {
        const listing = await listProjectFiles(project.id, directory);
        return listing.entries.map((entry) => entry.path);
      } catch {
        // A folder that cannot be listed cannot be checked; the core still
        // refuses to overwrite, so the upload fails safely rather than silently.
        return [];
      }
    },
    [project.id],
  );

  // ---------------------------------------------------------------- opening
  const openPath = useCallback(
    async (path: string) => {
      try {
        const file = await readProjectFile(project.id, path);
        setEditor((current) => openFile(current, file));
        setSelection(selectOnly(path));
        setTargetDirectory(parentOf(path));
        setHistory((current) => {
          if (current.paths[current.index] === path) return current;
          const paths = [...current.paths.slice(0, current.index + 1), path];
          return { paths, index: paths.length - 1 };
        });
      } catch (error) {
        // A binary or oversized file is refused by the core with a readable
        // reason. Shown as-is rather than opening a garbled buffer.
        report('Files', error);
      }
    },
    [project.id, report],
  );

  const save = useCallback(
    async (path: string) => {
      const target = bufferFor(editor, path);
      if (!target || target.readOnly) return;

      const written = target.current;
      setSaving(true);
      try {
        await writeProjectFile(project.id, path, written);
        // The text that was written, not what is in the editor now: typing
        // during a save must leave the buffer dirty.
        setEditor((current) => markSaved(current, path, written));
        say('Files', `Saved ${path}.`);
      } catch (error) {
        report('Files', error);
      } finally {
        setSaving(false);
      }
    },
    [editor, project.id, report, say],
  );

  const requestClose = useCallback(
    (path: string) => {
      const target = bufferFor(editor, path);
      if (target && isDirty(target)) {
        setConfirm({
          title: `Do you want to save the changes you made to ${tabLabel(path)}?`,
          detail: 'Your changes will be lost if you do not save them.',
          confirmLabel: 'Save',
          onConfirm: () => {
            setConfirm(null);
            void save(path).then(() => setEditor((current) => closeFile(current, path)));
          },
        });
        return;
      }
      setEditor((current) => closeFile(current, path));
    },
    [editor, save],
  );

  // ------------------------------------------------------------- file edits
  const beginCreate = useCallback(
    (directory: string, isFolder: boolean) => {
      showSidebar('explorer');
      setExpanded((current) => (current.includes(directory) ? current : [...current, directory]));
      if (directory && !listings[directory]) void loadDirectory(directory);
      setEditing({
        kind: isFolder ? 'create-folder' : 'create-file',
        path: directory,
        initialValue: '',
      });
    },
    [listings, loadDirectory, showSidebar],
  );

  const beginRename = useCallback((entry: { path: string; name: string }) => {
    setEditing({ kind: 'rename', path: entry.path, initialValue: entry.name });
  }, []);

  const submitEdit = useCallback(
    async (value: string) => {
      const request = editing;
      setEditing(null);
      if (!request) return;

      try {
        if (request.kind === 'rename') {
          const renamed = await renameProjectFile(project.id, request.path, value);
          await loadDirectory(parentOf(request.path));
          // The open tab follows, keeping any unsaved text.
          setEditor((current) => renamePath(current, request.path, renamed.path));
          setSelection((current) => renameInSelection(current, request.path, renamed.path));
          say('Files', `Renamed ${request.path} to ${renamed.path}.`);
          return;
        }

        const isFolder = request.kind === 'create-folder';
        const path = childPath(request.path, value);
        const created = await createProjectFile(project.id, path, isFolder);
        await loadDirectory(request.path);
        say('Files', `Created ${created.path}.`);
        if (isFolder) {
          setExpanded((current) => (current.includes(path) ? current : [...current, path]));
          setTargetDirectory(path);
          setSelection(selectOnly(path));
        } else {
          await openPath(created.path);
        }
      } catch (error) {
        report('Files', error);
      }
    },
    [editing, loadDirectory, openPath, project.id, report, say],
  );

  /**
   * Delete every path given, behind one confirmation.
   *
   * One dialog for the whole selection rather than one per file: seven prompts
   * in a row is how people learn to click through them without reading.
   * Failures are collected and reported together for the same reason.
   */
  const removePaths = useCallback(
    (paths: string[]) => {
      if (paths.length === 0) return;
      const entries = paths
        .map((path) => findEntry(listingsRef.current, path))
        .filter((entry): entry is FileEntry => entry !== null);
      if (entries.length === 0) return;

      const [only] = entries;
      const title =
        entries.length === 1 && only
          ? only.kind === 'directory'
            ? `Are you sure you want to delete “${only.name}” and everything inside it?`
            : `Are you sure you want to delete “${only.name}”?`
          : `Are you sure you want to delete these ${entries.length} items?`;

      setConfirm({
        title,
        detail: 'This cannot be undone.',
        confirmLabel: 'Delete',
        danger: true,
        onConfirm: () => {
          setConfirm(null);
          void (async () => {
            const failed: string[] = [];
            const directories = new Set<string>();

            for (const entry of entries) {
              try {
                await deleteProjectFile(project.id, entry.path, entry.kind === 'directory');
                directories.add(parentOf(entry.path));
                setEditor((current) => forgetDeleted(current, entry.path));
                say('Files', `Deleted ${entry.path}.`);
              } catch (error) {
                failed.push(`${entry.path}: ${errorMessage(error)}`);
              }
            }

            await Promise.all([...directories].map((directory) => loadDirectory(directory)));
            const gone = new Set(
              entries
                .filter((entry) => !failed.some((line) => line.startsWith(entry.path)))
                .map((entry) => entry.path),
            );
            setSelection((current) =>
              pruneSelection(
                current,
                current.selected.filter((path) => !gone.has(path)),
              ),
            );

            if (failed.length > 0) {
              say(
                'Files',
                `Could not delete ${failed.length} item(s): ${failed.join('; ')}`,
                'error',
              );
            }
          })();
        },
      });
    },
    [loadDirectory, project.id, say],
  );

  const duplicate = useCallback(
    async (entry: FileEntry) => {
      const directory = parentOf(entry.path);
      const existing = await pathsIn(directory);
      const dot = entry.name.lastIndexOf('.');
      const stem = dot > 0 ? entry.name.slice(0, dot) : entry.name;
      const extension = dot > 0 ? entry.name.slice(dot) : '';

      let candidate = childPath(directory, `${stem} copy${extension}`);
      for (let counter = 2; existing.includes(candidate); counter += 1) {
        candidate = childPath(directory, `${stem} copy ${counter}${extension}`);
      }

      try {
        const copied = await copyProjectFile(project.id, entry.path, candidate);
        await loadDirectory(directory);
        say('Files', `Copied ${entry.path} to ${copied.path}.`);
      } catch (error) {
        report('Files', error);
      }
    },
    [loadDirectory, pathsIn, project.id, report, say],
  );

  /** What is at a destination, and whether each entry is a folder. */
  const kindsIn = useCallback(
    async (directory: string): Promise<Map<string, ItemKind>> => {
      try {
        const listing = await listProjectFiles(project.id, directory);
        return new Map(
          listing.entries.map((entry) => [
            entry.path,
            entry.kind === 'directory' ? ('directory' as const) : ('file' as const),
          ]),
        );
      } catch {
        // Unreadable: the operation will fail for its own reasons and say so.
        return new Map();
      }
    },
    [project.id],
  );

  /**
   * Put the conflicts in front of the user and wait for an answer.
   *
   * Three answers, not two: confirm, cancel, or "send this somewhere else",
   * which is not an answer at all but a new question. The caller rewrites the
   * plan and asks again, which is why relocation cannot be resolved inside the
   * dialog — only the caller knows how to re-plan its own operation.
   */
  const reviewConflicts = useCallback(
    (
      conflicts: Conflict[],
      operation: Operation,
      existing: string[],
      context: {
        items: PlannedItem[];
        existingKinds: Map<string, ItemKind>;
        grouping?: PlanGrouping;
        directories?: string[];
        initialDecisions?: Decisions;
        notice?: string | null;
      },
    ) =>
      new Promise<ConflictOutcome>((resolve) => {
        setConflictReview((current) => ({
          conflicts,
          operation,
          existing,
          items: context.items,
          existingKinds: context.existingKinds,
          grouping: context.grouping ?? noGrouping,
          directories: context.directories ?? [],
          initialDecisions: context.initialDecisions,
          notice: context.notice ?? null,
          revision: (current?.revision ?? 0) + 1,
          resolve: (outcome) => {
            setConflictReview(null);
            resolve(outcome);
          },
        }));
      }),
    [],
  );

  /**
   * Carry out resolved work.
   *
   * A replacement is staged rather than destroyed: the existing item is renamed
   * aside, the incoming one is put in place, and only then is the old one
   * deleted. If anything fails, the old one is renamed back — so a failed
   * replace leaves exactly what was there before, not a hole.
   *
   * A move is a rename, so the source only stops existing once the destination
   * does. Nothing is copied-then-deleted.
   */
  const executeResolved = useCallback(
    async (items: ResolvedItem[], operation: Operation): Promise<string[]> => {
      const failures: string[] = [];
      const landed: string[] = [];
      const touched = new Set<string>();

      for (const item of items) {
        // A unique sibling name; the core refuses a rename with a separator in
        // it, so the backup stays in the same folder as its original.
        const backup = `${item.destination}.replaced-${createUploadId().slice(0, 8)}`;
        let staged = false;

        try {
          if (item.replaces) {
            await renameProjectFile(project.id, item.destination, nameOfPath(backup));
            staged = true;
          }

          if (operation === 'copy') {
            const copied = await copyProjectFile(project.id, item.source, item.destination);
            landed.push(copied.path);
          } else {
            const moved = await moveProjectFile(project.id, item.source, item.destination);
            touched.add(parentOf(item.source));
            setEditor((current) => renamePath(current, item.source, moved.path));
            landed.push(moved.path);
          }

          // Committed. Only now is the replaced item allowed to disappear.
          if (staged) {
            await deleteProjectFile(project.id, backup, item.replacedDirectory === true);
          }
        } catch (error) {
          if (staged) {
            try {
              await renameProjectFile(project.id, backup, nameOfPath(item.destination));
            } catch (restoreError) {
              // The one case worth shouting about: the original is still on
              // disk but under a name nobody asked for.
              failures.push(
                `${item.destination}: could not be replaced and its original is left at ${backup} (${errorMessage(restoreError)})`,
              );
              continue;
            }
          }
          failures.push(`${item.source}: ${errorMessage(error)}`);
        }
        touched.add(parentOf(item.destination));
      }

      await Promise.all([...touched].map((directory) => loadDirectory(directory)));
      if (landed.length > 0) setSelection(selectPaths(landed));
      return failures;
    },
    [loadDirectory, project.id],
  );

  /**
   * Settle every collision, then carry the work out.
   *
   * Returns the failures, or `null` when the user cancelled — the caller must
   * tell those apart, because a cancelled paste has to keep the clipboard.
   *
   * The plan is analysed against the destination twice: once to build the
   * dialog, and again immediately before executing. A file that appeared while
   * the dialog was open sends the user back to review it rather than being
   * silently overwritten by a decision made about something else.
   */
  const settleAndExecute = useCallback(
    async (
      wanted: { source: string; destination: string; incoming: ItemKind }[],
      directory: string,
      operation: Operation,
    ): Promise<string[] | null> => {
      if (wanted.length === 0) return [];

      // Relocating rewrites the plan and asks again; it is not the destination
      // changing underneath us, so it must not spend the stale-retry budget.
      let items = wanted;
      let carried: Decisions | undefined;
      let notice: string | null = null;

      for (let attempt = 0; attempt < 3;) {
        const kinds = await kindsIn(directory);
        const analysis = analyse(items, kinds, operation);
        const conflicts = allConflicts(analysis);

        let decisions: Decisions = {};
        if (conflicts.length > 0) {
          const answered = await reviewConflicts(conflicts, operation, [...kinds.keys()], {
            items,
            existingKinds: kinds,
            directories: pickableDirectories(kinds.keys(), [directory]),
            initialDecisions: carried,
            notice,
          });

          if (answered.kind === 'cancelled') {
            say('Files', 'Cancelled.', 'warn');
            return null;
          }

          if (answered.kind === 'relocate') {
            const plan = planRelocation(conflicts, answered.request, {
              items,
              existing: kinds,
              grouping: noGrouping,
            });
            items = plan.items;

            // Re-analysed straight away, so any collision the new destination
            // creates is shown rather than discovered while writing.
            const after = allConflicts(analyse(items, kinds, operation));
            // The decisions the user had already made come back, minus the ones
            // about collisions that have just moved.
            carried = preserveDecisions(answered.decisions, conflicts, after, plan.moved);
            notice = describeRelocation(plan, after.length - conflicts.length);
            continue;
          }

          decisions = answered.decisions;
        }

        const resolved = resolvePlan(analysis, decisions, [...kinds.keys()]);
        if (resolved.items.length === 0) {
          if (resolved.skipped.length > 0) {
            say('Files', `${resolved.skipped.length} item(s) skipped.`);
          }
          return [];
        }

        // Checked against the destination as it is *now*, not as it was when
        // the dialog opened.
        const now = await pathsIn(directory);
        if (planIsStale(resolved, now)) {
          say('Files', 'The destination changed while you were deciding. Reviewing again.', 'warn');
          // Only a *stale* plan counts against the budget: the user relocating
          // things is progress, and cutting them off after three tries would be.
          attempt += 1;
          // The destinations the user picked are still wanted; only the
          // decisions about what changed underneath are re-asked.
          carried = undefined;
          notice = 'The project changed while you were deciding, so this has been checked again.';
          continue;
        }

        if (resolved.skipped.length > 0) {
          say('Files', `${resolved.skipped.length} item(s) skipped.`);
        }
        return executeResolved(resolved.items, operation);
      }

      say('Files', 'The destination kept changing; nothing was done.', 'error');
      return null;
    },
    [executeResolved, kindsIn, pathsIn, reviewConflicts, say],
  );

  /**
   * Move several paths into a folder.
   *
   * Planned before anything happens — a folder cannot go inside itself, an item
   * cannot land where it already is, and a name that is already taken is a
   * conflict rather than an overwrite. The plan is checked once for the whole
   * batch so a partial move is reported as one outcome, not seven.
   */
  const movePaths = useCallback(
    async (paths: string[], toDirectory: string) => {
      if (paths.length === 0) return;

      const existing = await pathsIn(toDirectory);
      const plan = planMove(paths, toDirectory, existing, (path) => {
        const entry = findEntry(listingsRef.current, path);
        return entry?.kind === 'directory';
      });

      for (const rejection of plan.rejected) {
        say('Files', rejection.reason, 'warn');
      }
      if (plan.items.length === 0) return;

      const failures = await settleAndExecute(
        plan.items.map((item) => ({
          source: item.from,
          destination: item.to,
          incoming: item.isDirectory ? ('directory' as const) : ('file' as const),
        })),
        toDirectory,
        'move',
      );
      if (failures === null) return;

      if (failures.length > 0) {
        // Named individually: a half-finished move is exactly when the user
        // needs to know which files did not arrive.
        say('Files', `Could not move ${failures.length} item(s): ${failures.join('; ')}`, 'error');
      } else {
        say('Files', describePaste(plan, toDirectory));
      }
    },
    [pathsIn, say, settleAndExecute],
  );

  /** Copy or cut the selection. Nothing happens until it is pasted. */
  const cutOrCopy = useCallback(
    (mode: 'copy' | 'cut') => {
      const paths = selectionRef.current.selected;
      if (paths.length === 0) return;
      setClipboard({ mode, paths: [...paths] });
      say(
        'Files',
        `${mode === 'cut' ? 'Cut' : 'Copied'} ${paths.length} item${paths.length === 1 ? '' : 's'}.`,
      );
    },
    [say],
  );

  /**
   * Paste the clipboard.
   *
   * A cut is carried out as a move, so the source only disappears once the
   * destination exists — the core renames rather than copying-then-deleting,
   * which is what makes that true rather than merely intended.
   */
  const paste = useCallback(async () => {
    const held = clipboardRef.current;
    if (!held) return;

    const destination = pasteDestination(
      selectionRef.current.selected,
      (path) => findEntry(listingsRef.current, path)?.kind === 'directory',
      targetDirectoryRef.current,
    );

    const existing = await pathsIn(destination);
    const plan = planPaste(held, destination, existing, (path) => {
      const entry = findEntry(listingsRef.current, path);
      return entry?.kind === 'directory';
    });

    for (const rejection of plan.rejected) say('Files', rejection.reason, 'warn');
    if (plan.items.length === 0) return;

    const failures = await settleAndExecute(
      plan.items.map((item) => ({
        source: item.from,
        destination: item.to,
        incoming: item.isDirectory ? ('directory' as const) : ('file' as const),
      })),
      destination,
      held.mode === 'cut' ? 'move' : 'copy',
    );
    if (failures === null) return;

    // A cut is spent once pasted; a copy can be pasted again. Only cleared
    // when the paste actually ran, so a cancelled dialog keeps the clipboard.
    if (held.mode === 'cut' && failures.length === 0) setClipboard(null);

    if (failures.length > 0) {
      say('Files', `Could not paste ${failures.length} item(s): ${failures.join('; ')}`, 'error');
    } else {
      say('Files', describePaste(plan, destination));
    }
  }, [pathsIn, say, settleAndExecute]);

  const copyToClipboard = useCallback(
    (text: string, description: string) => {
      navigator.clipboard
        .writeText(text)
        .then(() => say('Workspace', `Copied ${description}.`))
        .catch((error: unknown) => report('Workspace', error));
    },
    [report, say],
  );

  const reveal_ = useCallback(
    async (path: string) => {
      try {
        await revealProjectPath(project.id, path);
      } catch (error) {
        report('Files', error);
      }
    },
    [project.id, report],
  );

  // ---------------------------------------------------------------- uploads
  const updateUpload = useCallback((id: string, patch: UploadPatch) => {
    setUploads((current) => current.map((item) => (item.id === id ? { ...item, ...patch } : item)));
  }, []);

  const runUpload = useCallback(
    async (item: BrowserUploadItem) => {
      const throwIfCancelled = () => {
        if (cancelledUploads.current.has(item.id)) throw new Error(UPLOAD_CANCELLED);
      };

      updateUpload(item.id, { status: 'uploading', uploadedBytes: 0, message: 'Starting upload…' });

      try {
        throwIfCancelled();
        // A replacement deletes first: the core will not overwrite, and the
        // user has already been asked.
        if (item.replaces) {
          await deleteProjectFile(project.id, item.path, false);
        }

        await beginProjectFileUpload(project.id, item.path, item.uploadId, item.sizeBytes);

        let offset = 0;
        while (offset < item.sizeBytes) {
          throwIfCancelled();
          const chunk = item.file.slice(
            offset,
            Math.min(offset + UPLOAD_CHUNK_BYTES, item.sizeBytes),
          );
          const bytes = Array.from(new Uint8Array(await chunk.arrayBuffer()));
          throwIfCancelled();
          offset = await appendProjectFileUpload(
            project.id,
            item.path,
            item.uploadId,
            offset,
            bytes,
          );
          updateUpload(item.id, { uploadedBytes: offset, message: 'Uploading…' });
        }

        throwIfCancelled();
        const uploaded = await finishProjectFileUpload(
          project.id,
          item.path,
          item.uploadId,
          item.sizeBytes,
        );
        updateUpload(item.id, {
          status: 'success',
          uploadedBytes: item.sizeBytes,
          message: 'Uploaded.',
        });
        say('Transfers', `Uploaded ${uploaded.path}.`);
        await refreshBranch(uploaded.path);
      } catch (error) {
        await cancelProjectFileUpload(project.id, item.path, item.uploadId).catch(() => undefined);
        if (error instanceof Error && error.message === UPLOAD_CANCELLED) {
          updateUpload(item.id, { status: 'cancelled', message: 'Cancelled.' });
          return;
        }
        const message = errorMessage(error);
        updateUpload(item.id, { status: 'failed', message });
        say('Transfers', `${item.path}: ${message}`, 'error');
      }
    },
    [project.id, refreshBranch, say, updateUpload],
  );

  const runNativeImport = useCallback(
    async (item: NativeImportItem) => {
      updateUpload(item.id, {
        status: 'uploading',
        uploadedBytes: 0,
        sizeBytes: 0,
        copiedFiles: 0,
        totalFiles: 0,
        message: 'Preparing import…',
      });

      try {
        const imported = await importProjectFiles(
          project.id,
          item.targetDirectory,
          item.sourcePaths,
          item.uploadId,
          item.unwrapPaths ?? [],
        );
        updateUpload(item.id, { status: 'success', message: 'Imported.' });
        say(
          'Transfers',
          `Imported ${imported.length} item${imported.length === 1 ? '' : 's'} into ${displayDirectory(item.targetDirectory)}.`,
        );
        // Every imported top-level entry, plus the folder they landed in: an
        // unwrapped project adds many entries at once and the tree has to show
        // all of them, not just the first.
        await Promise.all(imported.map((entry) => refreshBranch(entry.path)));
        await loadDirectory(item.targetDirectory);
      } catch (error) {
        const message = errorMessage(error);
        if (cancelledUploads.current.has(item.id) || message.toLowerCase().includes('cancelled')) {
          updateUpload(item.id, { status: 'cancelled', message: 'Cancelled.' });
          return;
        }
        updateUpload(item.id, { status: 'failed', message });
        say('Transfers', message, 'error');
      }
    },
    [loadDirectory, project.id, refreshBranch, say, updateUpload],
  );

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;

    void onFileImportProgress((progress) => {
      if (progress.projectId !== project.id) return;

      // A batch belonging to an organised import feeds the operation instead
      // of standing on its own in the transfer list.
      const inFlight = activeImports.current.get(progress.importId);
      if (inFlight) {
        inFlight.apply({
          importId: progress.importId,
          copiedFiles: progress.copiedFiles,
          copiedBytes: progress.copiedBytes,
          totalFiles: progress.totalFiles,
          totalBytes: progress.totalBytes,
          currentPath: progress.currentPath,
        });
        return;
      }

      updateUpload(progress.importId, {
        uploadedBytes: progress.copiedBytes,
        sizeBytes: progress.totalBytes,
        copiedFiles: progress.copiedFiles,
        totalFiles: progress.totalFiles,
        message: progress.currentPath ? `Importing ${progress.currentPath}…` : 'Preparing import…',
      });
    })
      .then((next) => {
        if (active) unlisten = next;
        else next();
      })
      .catch((error: unknown) => report('Transfers', error));

    return () => {
      active = false;
      unlisten?.();
    };
  }, [project.id, report, updateUpload]);

  /** Ask about existing files, once for the whole drop. */
  const chooseConflictAction = useCallback(
    (paths: string[], directory: string): Promise<ConflictChoice | null> =>
      new Promise((resolve) => {
        setConflict({
          paths,
          directory,
          resolve: (choice) => {
            setConflict(null);
            resolve(choice);
          },
        });
      }),
    [],
  );

  const queueBrowserDrop = useCallback(
    (dropped: DroppedItems) => {
      const duplicatePath = duplicateDroppedFilePath(dropped.files);
      if (duplicatePath) {
        say(
          'Transfers',
          `The drop contains two files called "${duplicatePath}". Rename one and try again.`,
          'error',
        );
        showPanel('output');
        return;
      }

      const directories = directoryPathsForDrop(dropped.files, dropped.directories);
      const candidates = dropped.files
        .map((entry) => {
          const relativePath = cleanDropPath(entry.relativePath);
          if (!relativePath) return null;
          return { file: entry.file, path: childPath(targetDirectory, relativePath) };
        })
        .filter((entry): entry is { file: File; path: string } => entry !== null);

      if (candidates.length === 0 && directories.length === 0) {
        say('Transfers', 'No files were found in that drop.', 'warn');
        return;
      }

      void (async () => {
        try {
          // Only the files landing directly in the target can clash with
          // something already listed; a nested path lands in a folder this
          // drop is creating.
          const existing = await pathsIn(targetDirectory);
          const clashes = conflictingPaths(
            existing,
            candidates.map((entry) => entry.path),
          );

          let choice: ConflictChoice = 'rename';
          if (clashes.length > 0) {
            const chosen = await chooseConflictAction(clashes, targetDirectory);
            if (chosen === null) {
              say('Transfers', 'Import cancelled.', 'warn');
              return;
            }
            choice = chosen;
          }

          const { uploads: resolved, skipped } = resolveConflicts(existing, candidates, choice);
          if (skipped.length > 0) {
            say('Transfers', `Skipped ${skipped.length} file(s) that already exist.`, 'warn');
          }

          for (const directory of directories) {
            await createProjectFile(project.id, childPath(targetDirectory, directory), true).catch(
              () => undefined,
            );
          }

          const queued: BrowserUploadItem[] = resolved.map((entry) => {
            const id = createUploadId();
            return {
              kind: 'browser',
              id,
              uploadId: id,
              file: entry.item.file,
              path: entry.path,
              replaces: entry.replaces,
              uploadedBytes: 0,
              sizeBytes: entry.item.file.size,
              status: 'queued',
              message: 'Waiting to upload…',
            };
          });

          if (queued.length > 0) {
            setUploads((current) => [...current, ...queued]);
            for (const item of queued) void runUpload(item);
          } else if (directories.length > 0) {
            say(
              'Transfers',
              `Created ${directories.length} folder${directories.length === 1 ? '' : 's'} in ${displayDirectory(targetDirectory)}.`,
            );
          }
          await loadDirectory(targetDirectory);
        } catch (error) {
          report('Transfers', error);
        }
      })();
    },
    [
      chooseConflictAction,
      loadDirectory,
      pathsIn,
      project.id,
      report,
      runUpload,
      say,
      showPanel,
      targetDirectory,
    ],
  );

  /** Start the copy, once what to do with each path has been settled. */
  const startNativeImport = useCallback(
    (sourcePaths: string[], unwrapPaths: string[], directory: string) => {
      const id = createUploadId();
      const item: NativeImportItem = {
        kind: 'native',
        id,
        uploadId: id,
        sourcePaths,
        unwrapPaths,
        targetDirectory: directory,
        path: nativeImportLabel(sourcePaths, directory),
        uploadedBytes: 0,
        sizeBytes: 0,
        copiedFiles: 0,
        totalFiles: 0,
        status: 'queued',
        message: 'Waiting to import…',
      };
      setUploads((current) => [...current, item]);
      void runNativeImport(item);
    },
    [runNativeImport],
  );

  /**
   * A drop from the operating system.
   *
   * The window is handed paths and nothing else — it cannot read a directory —
   * so the core is asked what each one is before anything is copied. A folder
   * that turns out to be a project is imported as its *contents*: keeping the
   * folder would produce `MyProject/MyProject/package.json`, which is the whole
   * reason this path exists.
   *
   * The user is asked first whenever that reshaping is about to happen, or
   * whenever several things arrive at once. Dropping one ordinary folder is not
   * worth a dialog.
   */
  const queueNativeImport = useCallback(
    (paths: string[]) => {
      const sourcePaths = [...new Set(paths.filter((path) => path.trim().length > 0))];
      if (sourcePaths.length === 0) {
        say('Transfers', 'No files or folders were found in that drop.', 'warn');
        return;
      }

      const directory = targetDirectory;
      void (async () => {
        let candidates: ImportCandidate[];
        try {
          candidates = await inspectImportPaths(sourcePaths);
        } catch (error) {
          // Looking failed, so nothing is known about the shape of the drop.
          // Importing it unchanged is the behaviour that existed before any of
          // this and never loses a file.
          report('Transfers', error);
          startNativeImport(sourcePaths, [], directory);
          return;
        }

        const plan = planImport(candidates);
        for (const candidate of candidates.filter((item) => item.isDirectory)) {
          say('Transfers', `${candidate.name}: ${explainDetection(candidate)}`);
        }

        // Nothing surprising: one ordinary folder, or loose files. The fast
        // path, with no dialog at all.
        if (!plan.unwraps && plan.projects.length === 0) {
          say('Transfers', describePlan(plan, directory));
          startNativeImport(sourcePaths, [], directory);
          return;
        }

        // One clear project on its own gets the compact confirmation; anything
        // mixed or ambiguous gets the organiser, where it can be rearranged.
        const simple =
          plan.projects.length === 1 && plan.folders.length === 0 && plan.files.length === 0;
        if (simple) {
          setImportPreview({ plan, directory });
          return;
        }

        setOrganising({
          groups: groupsFrom(candidates, directory),
          candidates,
          directory,
          existing: await pathsIn(directory),
        });
      })();
    },
    [pathsIn, report, say, startNativeImport, targetDirectory],
  );

  /**
   * Run an organised import.
   *
   * The plan is checked against the project *again* here rather than trusting
   * the check made when the organiser opened: files can arrive while a dialog
   * is up, and a conflict check from a minute ago is not a conflict check.
   */
  /**
   * Carry out an approved organised import.
   *
   * One operation across every group: the totals are known before the first
   * byte moves, so the bar fills once rather than once per group. Each batch is
   * still a separate core call — that is how the core imports — but its
   * progress is folded into the operation rather than shown on its own.
   *
   * A replacement is staged: the existing item is renamed aside first and only
   * deleted once the import has committed. If the import fails it is renamed
   * back, and anything this operation created is removed. Files that were
   * already in the project and were not staged are never touched.
   */
  const executeOperation = useCallback(
    async (
      operationId: string,
      planned: PlannedDestination[],
      resolved: ResolvedPlan,
      groups: ImportGroup[],
      directory: string,
    ) => {
      const sizeOf = new Map(planned.map((entry) => [entry.source, entry]));
      const groupFor = new Map<string, string>();
      for (const group of groups) {
        for (const entry of group.entries) groupFor.set(entry.path, group.name);
      }

      // One batch per destination directory, carrying only the sources that
      // survived resolution, plus the names and replacements it decided.
      const byDestination = new Map<string, OperationBatch>();
      for (const item of resolved.items) {
        const cut = item.destination.lastIndexOf('/');
        const key = cut < 0 ? '' : item.destination.slice(0, cut);
        const batch =
          byDestination.get(key) ??
          ({
            importId: createUploadId(),
            groupName: groupFor.get(item.source) ?? 'Files',
            destination: key,
            sourcePaths: [],
            unwrapPaths: [],
            destinationNames: [],
            replacePaths: [],
            totalEntries: 0,
            totalBytes: 0,
          } satisfies OperationBatch);

        batch.sourcePaths.push(item.source);
        const finalName = item.destination.slice(cut + 1);
        const sourceName = baseName(item.source);
        if (finalName !== sourceName) batch.destinationNames.push([item.source, finalName]);
        if (item.replaces) batch.replacePaths.push(item.destination);

        const measured = sizeOf.get(item.source);
        batch.totalEntries += Math.max(1, measured?.totalFiles ?? 1);
        batch.totalBytes += measured?.totalBytes ?? 0;
        byDestination.set(key, batch);
      }

      const batches = [...byDestination.values()];
      let operation = newOperation(operationId, batches);

      const publish = (next: ImportOperationProgress, final = false) => {
        operation = next;
        // A stale operation must never repaint a newer one.
        if (operationRef.current !== operationId) return;
        const now = Date.now();
        if (!shouldRender(lastProgressRender.current, now, { final })) return;
        lastProgressRender.current = now;
        setOperationProgress(next);
      };

      publish(enterPhase(operation, 'preparing'), true);

      const done = { entries: 0, bytes: 0 };

      // The staging, commit and rollback live in `replacementTransaction`, with
      // the filesystem injected — that is the only way the rollback path can be
      // made to run on purpose in a test, and rollback is code that never runs
      // except when something has already gone wrong.
      const result = await runReplacementTransaction(
        batches,
        {
          rename: (path, toName) =>
            renameProjectFile(project.id, path, toName).then(() => undefined),
          remove: (path, isDirectory) =>
            deleteProjectFile(project.id, path, isDirectory).then(() => undefined),
          isDirectory: (path) => findEntry(listingsRef.current, path)?.kind === 'directory',
          importBatch: async (batch) => {
            activeImports.current.set(batch.importId, {
              operationId,
              batches,
              finished: { ...done },
              apply: (event) => publish(applyBatchProgress(operation, batches, { ...done }, event)),
            });
            try {
              const imported = await importProjectFiles(
                project.id,
                batch.destination,
                batch.sourcePaths,
                batch.importId,
                batch.unwrapPaths,
                batch.destinationNames,
              );
              return imported.map((entry) => entry.path);
            } finally {
              activeImports.current.delete(batch.importId);
            }
          },
        },
        {
          onPhase: (phase) => publish(enterPhase(operation, phase), true),
          onBatchComplete: (batch) => {
            done.entries += batch.totalEntries;
            done.bytes += batch.totalBytes;
            publish(completeBatch(operation, batches, batch.importId), true);
          },
          onRollbackStart: (total) => publish(startRollback(operation, total), true),
          onRollbackStep: (failed) => publish(advanceRollback(operation, failed), true),
          isCancelled: () => cancelledOperations.current.has(operationId),
        },
        `replaced-${operationId.slice(0, 8)}`,
      );

      const created = result.created;

      if (result.outcome === 'completed') {
        publish(enterCommit(operation), true);
      }
      for (const error of result.errors.filter((entry) => entry.rollback !== true)) {
        publish(addError(operation, error), true);
      }
      if (result.failure !== null) {
        publish(
          addError(operation, { path: directory || 'the project root', message: result.failure }),
          true,
        );
      }
      if (result.outcome !== 'completed') {
        publish(finish(operation, result.outcome === 'cancelled' ? 'completed' : 'failed'), true);
      }

      // Refreshed once, at the end: a tree that reshuffles three times during
      // an operation cannot be read.
      const outcome = operation.phase === 'failed' ? 'failed' : 'completed';
      publish(enterPhase(operation, 'finalising'), true);
      const directories = new Set<string>([
        directory,
        ...batches.map((batch) => batch.destination),
      ]);
      await Promise.all([...directories].map((entry) => loadDirectory(entry)));
      if (created.length > 0 && outcome === 'completed') setSelection(selectPaths(created));

      publish(finish(operation, outcome), true);
      cancelledOperations.current.delete(operationId);

      if (resolved.skipped.length > 0) {
        say('Transfers', `${resolved.skipped.length} item(s) were skipped.`);
      }
      say('Transfers', summarise(groups));
    },
    [loadDirectory, project.id, say],
  );

  /**
   * Run an organised import as one operation.
   *
   * The plan is turned into real destinations by the core — it is the only
   * side that can read a folder, and an unwrapped group lands its children
   * rather than itself. Those destinations go through the same conflict
   * analysis and the same dialog as paste, cut and drag; there is no second
   * reduced path.
   *
   * Everything is checked again immediately before execution, because a file
   * can appear while the dialog is open and a decision made about a different
   * file must never overwrite it.
   */
  const runOrganisedImport = useCallback(
    async (initialGroups: ImportGroup[], directory: string) => {
      const operationId = createUploadId();
      operationRef.current = operationId;

      // The groups are the plan. A destination chosen in the dialog edits them,
      // and the next pass re-plans from the core — so the change reaches the
      // backend rather than being patched onto paths on the way past.
      let groups = initialGroups;
      /** Decisions from the previous pass, with the conflicts they answered. */
      let carried: { decisions: Decisions; conflicts: Conflict[] } | undefined;
      let notice: string | null = null;

      for (let attempt = 0; attempt < 3;) {
        const plan = planFrom(groups);
        if (plan.batches.length === 0) {
          say('Transfers', 'Nothing is left to import.', 'warn');
          return;
        }

        // 1. What would this actually create, and how much does it weigh?
        const planned: PlannedDestination[] = [];
        try {
          for (const batch of plan.batches) {
            const destinations = await planImportDestinations(
              project.id,
              batch.destination,
              batch.sourcePaths,
              batch.unwrapPaths,
            );
            planned.push(...destinations);
          }
        } catch (error) {
          report('Transfers', error);
          return;
        }

        // 2. Analyse every landing path at once, before anything is written.
        const existing = new Map<string, ItemKind>();
        for (const entry of planned) {
          if (entry.existing !== null) {
            existing.set(entry.relative, entry.existing === 'directory' ? 'directory' : 'file');
          }
        }
        const items: PlannedItem[] = planned.map((entry) => ({
          source: entry.source,
          destination: entry.relative,
          incoming: entry.isDirectory ? ('directory' as const) : ('file' as const),
        }));
        const analysis = analyse(items, existing, 'import');
        const conflicts = allConflicts(analysis);

        // 3. The same dialog the other operations use.
        let decisions: Decisions = {};
        if (conflicts.length > 0) {
          const grouping = groupingFor(planned, groups);
          const answered = await reviewConflicts(conflicts, 'import', [...existing.keys()], {
            items,
            existingKinds: existing,
            grouping,
            directories: pickableDirectories(await pathsIn(directory), [directory]),
            // A decision only survives if it still answers the same collision
            // at the same path; the rest go back to unresolved.
            initialDecisions: carried
              ? preserveDecisions(carried.decisions, carried.conflicts, conflicts)
              : undefined,
            notice,
          });

          if (answered.kind === 'cancelled') {
            say('Transfers', 'Import cancelled. The organiser is unchanged.', 'warn');
            return;
          }

          if (answered.kind === 'relocate') {
            // The organiser's groups are edited, not the paths: re-planning
            // through the core is what recalculates every child path, which is
            // how a group keeps its own shape instead of being flattened.
            const targets = planRelocation(conflicts, answered.request, {
              items,
              existing,
              grouping,
            });
            const moved = relocateGroups(
              groups,
              planned,
              conflicts,
              answered.request,
              targets.moved,
            );
            groups = moved.groups;
            carried = { decisions: answered.decisions, conflicts };
            notice = [moved.summary, ...moved.refused.map((entry) => entry.message)].join(' ');
            say('Transfers', moved.summary);
            continue;
          }

          decisions = answered.decisions;
        }

        const resolved = resolvePlan(analysis, decisions, [...existing.keys()]);
        if (resolved.items.length === 0) {
          say('Transfers', 'Everything was skipped; nothing was imported.', 'warn');
          return;
        }

        // 4. Revalidate. A destination that changed sends us back to review,
        // with the destinations the user chose still in the plan — those live
        // in `groups`, so a stale filesystem does not undo them.
        const now = await pathsIn(directory);
        if (planIsStale(resolved, now)) {
          say(
            'Transfers',
            'The project changed while you were deciding. Reviewing the conflicts again.',
            'warn',
          );
          attempt += 1;
          carried = { decisions, conflicts };
          notice =
            'The project changed while you were deciding, so the conflicts have been checked again.';
          continue;
        }

        await executeOperation(operationId, planned, resolved, groups, directory);
        return;
      }

      say('Transfers', 'The project kept changing; nothing was imported.', 'error');
    },
    [executeOperation, pathsIn, project.id, report, reviewConflicts, say],
  );

  const cancelUpload = useCallback(
    (item: UploadItem) => {
      cancelledUploads.current.add(item.id);
      updateUpload(item.id, { status: 'cancelled', message: 'Cancelling…' });
      const cancel =
        item.kind === 'native'
          ? cancelProjectFileImport(item.uploadId)
          : cancelProjectFileUpload(project.id, item.path, item.uploadId);
      void cancel.catch((error: unknown) => {
        updateUpload(item.id, { status: 'failed', message: errorMessage(error) });
      });
    },
    [project.id, updateUpload],
  );

  const retryUpload = useCallback(
    (item: UploadItem) => {
      const uploadId = createUploadId();
      const next: UploadItem = {
        ...item,
        id: uploadId,
        uploadId,
        uploadedBytes: 0,
        sizeBytes: item.kind === 'native' ? 0 : item.sizeBytes,
        copiedFiles: item.kind === 'native' ? 0 : item.copiedFiles,
        totalFiles: item.kind === 'native' ? 0 : item.totalFiles,
        status: 'queued',
        message: item.kind === 'native' ? 'Waiting to import…' : 'Waiting to upload…',
      };
      cancelledUploads.current.delete(item.id);
      cancelledUploads.current.delete(item.uploadId);
      setUploads((current) => current.map((upload) => (upload.id === item.id ? next : upload)));
      if (next.kind === 'native') void runNativeImport(next);
      else void runUpload(next);
    },
    [runNativeImport, runUpload],
  );

  // ----------------------------------------------------------- drag and drop
  /**
   * The OS drag/drop event arrives for the whole window, so every native drag
   * is hit-tested against the explorer here. Anywhere else — the editor, the
   * panel, the tab strip — is not a drop target and the drag is ignored.
   */
  const overExplorer = useCallback((position: DropPoint) => {
    const zone = explorer.current;
    if (!zone) return false;
    return isInsideDropZone(position, zone.getBoundingClientRect(), window.devicePixelRatio);
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let active = true;

    void getCurrentWebviewWindow()
      .onDragDropEvent((event) => {
        switch (event.payload.type) {
          case 'enter':
          case 'over': {
            const inside = overExplorer(event.payload.position);
            dragDepth.current = inside ? 1 : 0;
            setDraggingOver(inside);
            break;
          }
          case 'drop': {
            const inside = overExplorer(event.payload.position);
            dragDepth.current = 0;
            setDraggingOver(false);
            // Dropped on the editor or the chrome: not a target, and importing
            // it would be a surprise the user cannot undo.
            if (inside) queueNativeImport(event.payload.paths);
            break;
          }
          case 'leave':
            dragDepth.current = 0;
            setDraggingOver(false);
            break;
        }
      })
      .then((next) => {
        if (active) {
          unlisten = next;
          nativeListenerReady.current = true;
        } else {
          next();
        }
      })
      // Registration failing is not fatal: it means this window is not a Tauri
      // webview, and the HTML5 handlers below take over instead.
      .catch((error: unknown) => report('Transfers', error));

    return () => {
      active = false;
      nativeListenerReady.current = false;
      unlisten?.();
    };
  }, [overExplorer, queueNativeImport, report]);

  // The four below are inert while the OS listener is live, which drives the
  // highlight itself and hit-tests the pointer properly. `preventDefault` still
  // runs unconditionally: without it the webview navigates away to the dropped
  // file, whichever path is doing the importing.
  const onDragEnter = useCallback((event: React.DragEvent<HTMLElement>) => {
    event.preventDefault();
    if (isInternalDrag(event) || !shouldImportBrowserDrop(nativeListenerReady.current)) return;
    dragDepth.current += 1;
    setDraggingOver(true);
  }, []);

  const onDragOver = useCallback((event: React.DragEvent<HTMLElement>) => {
    event.preventDefault();
    if (!isInternalDrag(event)) event.dataTransfer.dropEffect = 'copy';
  }, []);

  const onDragLeave = useCallback((event: React.DragEvent<HTMLElement>) => {
    event.preventDefault();
    if (isInternalDrag(event) || !shouldImportBrowserDrop(nativeListenerReady.current)) return;
    dragDepth.current = Math.max(0, dragDepth.current - 1);
    if (dragDepth.current === 0) setDraggingOver(false);
  }, []);

  const onDrop = useCallback(
    (event: React.DragEvent<HTMLElement>) => {
      event.preventDefault();
      // A drag that started inside the tree is a move, and the row it landed
      // on has already handled it.
      if (isInternalDrag(event)) return;
      // The OS listener has already imported this drop, hit-tested against the
      // explorer. Doing it again here would import everything twice.
      if (!shouldImportBrowserDrop(nativeListenerReady.current)) return;

      dragDepth.current = 0;
      setDraggingOver(false);
      collectDroppedItems(event.dataTransfer)
        .then(queueBrowserDrop)
        .catch((error: unknown) => report('Transfers', error));
    },
    [queueBrowserDrop, report],
  );

  // ------------------------------------------------------- running the project
  const runProject = useCallback(
    async (action: RunAction) => {
      setBusyAction(action);
      showPanel('terminal');
      const verbs: Record<RunAction, string> = {
        start: 'Starting',
        restart: 'Restarting',
        stop: 'Stopping',
        kill: 'Killing',
      };
      say('Project', `${verbs[action]} ${project.slug}…`);

      // Starting is gated on the machine having the project's language;
      // stopping and killing never are, because neither needs it.
      const call = {
        start: guard(startProject),
        restart: guard(restartProject),
        stop: stopProject,
        kill: killProject,
      }[action];

      try {
        await call(project.id);
        await onRefreshProjects();
        say('Project', `${project.slug}: ${action} finished.`);
      } catch (error) {
        // Declining an install is an answer, not a failure to report.
        if (isDeclined(error)) say('Project', `${project.slug}: ${action} cancelled.`);
        else report('Project', error);
      } finally {
        setBusyAction(null);
      }
    },
    [guard, onRefreshProjects, project.id, project.slug, report, say, showPanel],
  );

  // --------------------------------------------------------------- searching
  const search = useMemo(() => searchProjectFiles, []);
  const sidebarSearch = useFileSearch(project.id, searchQuery, search, errorMessage);
  const quickOpen = useFileSearch(project.id, quickOpenTerm, search, errorMessage);

  const quickOpenPaths = useMemo(() => {
    if (quickOpenTerm.trim().length === 0) return editor.buffers.map((item) => item.path);
    return quickOpen.results.filter((entry) => entry.kind === 'file').map((entry) => entry.path);
  }, [editor.buffers, quickOpen.results, quickOpenTerm]);

  // --------------------------------------------------------------- commands
  const goTo = useCallback(
    (path: string, line: number, column: number) => {
      const jump = () => setReveal({ line, column, nonce: Date.now() });
      if (editor.active === path) {
        jump();
        return;
      }
      void openPath(path).then(jump);
    },
    [editor.active, openPath],
  );

  const commands = useMemo<Command[]>(() => {
    const hasFile = buffer !== null;
    const running = project.status === 'RUNNING';

    return [
      {
        id: 'file.new',
        title: 'New File…',
        category: 'File',
        keybinding: 'Ctrl+N',
        run: () => beginCreate(targetDirectory, false),
      },
      {
        id: 'file.newFolder',
        title: 'New Folder…',
        category: 'File',
        run: () => beginCreate(targetDirectory, true),
      },
      {
        id: 'file.save',
        title: 'Save',
        category: 'File',
        keybinding: 'Ctrl+S',
        enabled: hasFile && !buffer.readOnly,
        reason: hasFile ? 'This project is read-only right now' : 'No file is open',
        run: () => {
          if (buffer) void save(buffer.path);
        },
      },
      {
        id: 'file.saveAll',
        title: 'Save All',
        category: 'File',
        enabled: dirtyCount > 0,
        reason: 'Nothing has unsaved changes',
        run: () => {
          for (const item of editor.buffers.filter(isDirty)) void save(item.path);
        },
      },
      {
        id: 'file.revealExplorer',
        title: 'Reveal in File Explorer',
        category: 'File',
        enabled: hasFile,
        reason: 'No file is open',
        run: () => {
          if (buffer) void reveal_(buffer.path);
        },
      },
      {
        id: 'file.copyPath',
        title: 'Copy Path of Active File',
        category: 'File',
        enabled: hasFile,
        reason: 'No file is open',
        run: () => {
          if (buffer) copyToClipboard(absolutePath(projectRoot, buffer.path), 'the path');
        },
      },
      {
        id: 'file.close',
        title: 'Close Editor',
        category: 'File',
        keybinding: 'Ctrl+W',
        enabled: hasFile,
        reason: 'No file is open',
        run: () => {
          if (buffer) requestClose(buffer.path);
        },
      },
      {
        id: 'file.closeAll',
        title: 'Close All Editors',
        category: 'File',
        enabled: editor.buffers.length > 0,
        reason: 'No file is open',
        run: () => setEditor(closeAll()),
      },
      {
        id: 'view.toggleSidebar',
        title: 'Toggle Primary Side Bar',
        category: 'View',
        keybinding: 'Ctrl+B',
        run: () => patchLayout({ sidebarVisible: !layout.sidebarVisible }),
      },
      {
        id: 'view.togglePanel',
        title: 'Toggle Panel',
        category: 'View',
        keybinding: 'Ctrl+J',
        run: () => patchLayout({ panelVisible: !layout.panelVisible }),
      },
      {
        id: 'view.terminal',
        title: 'Toggle Terminal',
        category: 'View',
        keybinding: 'Ctrl+`',
        run: () =>
          patchLayout({
            panelVisible: !(layout.panelVisible && layout.panelTab === 'terminal'),
            panelTab: 'terminal',
          }),
      },
      {
        id: 'view.problems',
        title: 'Show Problems',
        category: 'View',
        keybinding: 'Ctrl+Shift+M',
        run: () => showPanel('problems'),
      },
      {
        id: 'view.output',
        title: 'Show Output',
        category: 'View',
        run: () => showPanel('output'),
      },
      {
        id: 'view.explorer',
        title: 'Show Explorer',
        category: 'View',
        keybinding: 'Ctrl+Shift+E',
        run: () => showSidebar('explorer'),
      },
      {
        id: 'view.search',
        title: 'Search Files by Name',
        category: 'View',
        keybinding: 'Ctrl+Shift+F',
        run: () => showSidebar('search'),
      },
      {
        id: 'explorer.refresh',
        title: 'Refresh Explorer',
        category: 'Explorer',
        run: () => {
          for (const directory of ['', ...expanded]) void loadDirectory(directory);
          say('Files', 'Refreshed the explorer.');
        },
      },
      {
        id: 'explorer.collapse',
        title: 'Collapse Folders in Explorer',
        category: 'Explorer',
        run: () => setExpanded([]),
      },
      {
        id: 'project.start',
        title: 'Start Project',
        category: 'Run',
        enabled: !running && busyAction === null,
        reason: 'The project is already running',
        run: () => void runProject('start'),
      },
      {
        id: 'project.stop',
        title: 'Stop Project',
        category: 'Run',
        enabled: running && busyAction === null,
        reason: 'The project is not running',
        run: () => void runProject('stop'),
      },
      {
        id: 'project.restart',
        title: 'Restart Project',
        category: 'Run',
        enabled: running && busyAction === null,
        reason: 'The project is not running',
        run: () => void runProject('restart'),
      },
      {
        id: 'project.kill',
        title: 'Kill Project',
        category: 'Run',
        enabled: running && busyAction === null,
        reason: 'The project is not running',
        run: () => void runProject('kill'),
      },
      {
        id: 'project.close',
        title: 'Close Project',
        category: 'Go',
        run: leave,
      },
      {
        id: 'app.settings',
        title: 'Open Settings',
        category: 'Preferences',
        run: onOpenSettings,
      },
    ];
  }, [
    beginCreate,
    buffer,
    busyAction,
    copyToClipboard,
    dirtyCount,
    editor.buffers,
    expanded,
    layout.panelTab,
    layout.panelVisible,
    layout.sidebarVisible,
    loadDirectory,
    onOpenSettings,
    patchLayout,
    project.status,
    projectRoot,
    requestClose,
    reveal_,
    runProject,
    save,
    say,
    showPanel,
    showSidebar,
    targetDirectory,
  ]);

  function leave() {
    if (dirtyCount > 0) {
      setConfirm({
        title: `${dirtyCount} file${dirtyCount === 1 ? '' : 's'} ${dirtyCount === 1 ? 'has' : 'have'} unsaved changes.`,
        detail: 'Closing the project will discard them.',
        confirmLabel: 'Close anyway',
        danger: true,
        onConfirm: () => {
          setConfirm(null);
          onLeave();
        },
      });
      return;
    }
    onLeave();
  }

  // Held in a ref so the shortcut handler and the toolbar buttons can run a
  // command by id without every one of them re-binding on each render.
  const commandsRef = useRef(commands);
  commandsRef.current = commands;

  const runCommand = useCallback((id: string) => {
    // Looked up at call time so the command's own `enabled` is the current one.
    const command = commandsRef.current.find((entry) => entry.id === id);
    if (command && command.enabled !== false) command.run();
  }, []);

  // ------------------------------------------------------------- shortcuts
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const modifier = event.ctrlKey || event.metaKey;
      const typing = isTypingTarget(event.target);

      if (modifier && !event.shiftKey && event.key.toLowerCase() === 's') {
        event.preventDefault();
        if (editor.active !== null) void save(editor.active);
        return;
      }
      if (modifier && event.shiftKey && event.key.toLowerCase() === 'p') {
        event.preventDefault();
        setPalette('commands');
        return;
      }
      if (modifier && !event.shiftKey && event.key.toLowerCase() === 'p') {
        event.preventDefault();
        setPalette('files');
        return;
      }
      if (modifier && !event.shiftKey && event.key.toLowerCase() === 'b') {
        event.preventDefault();
        runCommand('view.toggleSidebar');
        return;
      }
      if (modifier && !event.shiftKey && event.key.toLowerCase() === 'j') {
        event.preventDefault();
        runCommand('view.togglePanel');
        return;
      }
      if (modifier && event.key === '`') {
        event.preventDefault();
        runCommand('view.terminal');
        return;
      }
      if (modifier && !event.shiftKey && event.key.toLowerCase() === 'w') {
        event.preventDefault();
        if (editor.active !== null) requestClose(editor.active);
        return;
      }
      if (modifier && !event.shiftKey && event.key.toLowerCase() === 'n') {
        event.preventDefault();
        runCommand('file.new');
        return;
      }
      if (modifier && event.shiftKey && event.key.toLowerCase() === 'f') {
        event.preventDefault();
        runCommand('view.search');
        return;
      }
      if (modifier && event.shiftKey && event.key.toLowerCase() === 'e') {
        event.preventDefault();
        runCommand('view.explorer');
        return;
      }
      if (modifier && event.shiftKey && event.key.toLowerCase() === 'm') {
        event.preventDefault();
        runCommand('view.problems');
        return;
      }

      // Everything below acts on the tree, so it stands down whenever the user
      // is typing — in a rename box, the editor, a dialog or a text field.
      // Delete inside the editor deletes a character and must go on doing so.
      if (typing || isEditableTarget(event.target)) return;

      const key = event.key.toLowerCase();
      const current = selectionRef.current;
      const rows = visibleRef.current;

      if (modifier && !event.shiftKey && key === 'a') {
        event.preventDefault();
        setSelection(selectAll(rows));
        return;
      }
      if (modifier && !event.shiftKey && key === 'c') {
        event.preventDefault();
        cutOrCopy('copy');
        return;
      }
      if (modifier && !event.shiftKey && key === 'x') {
        event.preventDefault();
        cutOrCopy('cut');
        return;
      }
      if (modifier && !event.shiftKey && key === 'v') {
        event.preventDefault();
        void paste();
        return;
      }

      if (event.key === 'Escape') {
        setSelection(clearSelection());
        return;
      }

      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        const mode = event.shiftKey ? 'extend' : modifier ? 'keep' : 'replace';
        setSelection(moveFocus(current, rows, event.key === 'ArrowDown' ? 1 : -1, mode));
        return;
      }
      if (event.key === 'Home' || event.key === 'End') {
        event.preventDefault();
        const mode = event.shiftKey ? 'extend' : modifier ? 'keep' : 'replace';
        setSelection(focusEdge(current, rows, event.key === 'Home' ? 'first' : 'last', mode));
        return;
      }

      const focusedEntry =
        current.focused === null ? null : findEntry(listingsRef.current, current.focused);

      if (event.key === 'ArrowRight' && focusedEntry?.kind === 'directory') {
        event.preventDefault();
        if (!expandedRef.current.includes(focusedEntry.path)) {
          setExpanded((entries) => toggleExpanded(entries, focusedEntry.path));
          if (!listingsRef.current[focusedEntry.path]) void loadDirectory(focusedEntry.path);
        }
        return;
      }
      if (event.key === 'ArrowLeft' && focusedEntry?.kind === 'directory') {
        event.preventDefault();
        if (expandedRef.current.includes(focusedEntry.path)) {
          setExpanded((entries) => toggleExpanded(entries, focusedEntry.path));
        }
        return;
      }

      if (event.key === 'Enter' && focusedEntry) {
        event.preventDefault();
        if (focusedEntry.kind === 'directory') {
          setExpanded((entries) => toggleExpanded(entries, focusedEntry.path));
          if (!listingsRef.current[focusedEntry.path]) void loadDirectory(focusedEntry.path);
        } else if (focusedEntry.kind === 'file') {
          void openPath(focusedEntry.path);
        }
        return;
      }

      if (event.key === 'F2' && focusedEntry) {
        event.preventDefault();
        beginRename(focusedEntry);
        return;
      }
      if (event.key === 'Delete' && current.selected.length > 0) {
        event.preventDefault();
        removePaths(current.selected);
      }
    }

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [
    beginRename,
    cutOrCopy,
    editor.active,
    loadDirectory,
    openPath,
    paste,
    removePaths,
    requestClose,
    runCommand,
    save,
  ]);

  // ------------------------------------------------------------------ menus
  function commandEntry(id: string): MenuEntry {
    const command = commands.find((entry) => entry.id === id);
    if (!command) return { id, separator: true };
    return {
      id: command.id,
      label: command.title,
      keybinding: command.keybinding,
      enabled: command.enabled,
      run: command.run,
    };
  }

  const menus: Menu[] = [
    {
      id: 'file',
      label: 'File',
      entries: [
        commandEntry('file.new'),
        commandEntry('file.newFolder'),
        { id: 'file.sep1', separator: true },
        commandEntry('file.save'),
        commandEntry('file.saveAll'),
        { id: 'file.sep2', separator: true },
        commandEntry('file.revealExplorer'),
        commandEntry('file.copyPath'),
        { id: 'file.sep3', separator: true },
        commandEntry('file.close'),
        commandEntry('file.closeAll'),
        commandEntry('project.close'),
      ],
    },
    {
      id: 'edit',
      label: 'Edit',
      entries: [
        editorAction('edit.undo', 'Undo', 'Ctrl+Z', 'undo'),
        editorAction('edit.redo', 'Redo', 'Ctrl+Y', 'redo'),
        { id: 'edit.sep1', separator: true },
        editorAction('edit.cut', 'Cut', 'Ctrl+X', 'editor.action.clipboardCutAction'),
        editorAction('edit.copy', 'Copy', 'Ctrl+C', 'editor.action.clipboardCopyAction'),
        editorAction('edit.paste', 'Paste', 'Ctrl+V', 'editor.action.clipboardPasteAction'),
        { id: 'edit.sep2', separator: true },
        editorAction('edit.find', 'Find', 'Ctrl+F', 'actions.find'),
        editorAction('edit.replace', 'Replace', 'Ctrl+H', 'editor.action.startFindReplaceAction'),
      ],
    },
    {
      id: 'selection',
      label: 'Selection',
      entries: [
        editorAction('selection.all', 'Select All', 'Ctrl+A', 'editor.action.selectAll'),
        editorAction(
          'selection.expand',
          'Expand Selection',
          'Shift+Alt+Right',
          'editor.action.smartSelect.expand',
        ),
        { id: 'selection.sep', separator: true },
        editorAction(
          'selection.copyUp',
          'Copy Line Up',
          'Shift+Alt+Up',
          'editor.action.copyLinesUpAction',
        ),
        editorAction(
          'selection.moveDown',
          'Move Line Down',
          'Alt+Down',
          'editor.action.moveLinesDownAction',
        ),
      ],
    },
    {
      id: 'view',
      label: 'View',
      entries: [
        commandEntry('view.explorer'),
        commandEntry('view.search'),
        { id: 'view.sep1', separator: true },
        commandEntry('view.problems'),
        commandEntry('view.output'),
        commandEntry('view.terminal'),
        { id: 'view.sep2', separator: true },
        commandEntry('view.toggleSidebar'),
        commandEntry('view.togglePanel'),
      ],
    },
    {
      id: 'go',
      label: 'Go',
      entries: [
        {
          id: 'go.file',
          label: 'Go to File…',
          keybinding: 'Ctrl+P',
          run: () => setPalette('files'),
        },
        {
          id: 'go.command',
          label: 'Show All Commands',
          keybinding: 'Ctrl+Shift+P',
          run: () => setPalette('commands'),
        },
        { id: 'go.sep', separator: true },
        editorAction('go.line', 'Go to Line…', 'Ctrl+G', 'editor.action.gotoLine'),
        { id: 'go.sep2', separator: true },
        {
          id: 'go.back',
          label: 'Back',
          enabled: history.index > 0,
          run: () => navigate(-1),
        },
        {
          id: 'go.forward',
          label: 'Forward',
          enabled: history.index < history.paths.length - 1,
          run: () => navigate(1),
        },
      ],
    },
    {
      id: 'run',
      label: 'Run',
      entries: [
        commandEntry('project.start'),
        commandEntry('project.restart'),
        commandEntry('project.stop'),
        commandEntry('project.kill'),
      ],
    },
    {
      id: 'terminal',
      label: 'Terminal',
      entries: [commandEntry('view.terminal'), commandEntry('view.output')],
    },
    {
      id: 'help',
      label: 'Help',
      entries: [
        {
          id: 'help.version',
          label: `Version ${status?.appVersion ?? '—'}`,
          enabled: false,
          run: () => {},
        },
        commandEntry('app.settings'),
        {
          id: 'help.update',
          label:
            update.check?.state === 'available'
              ? `Install update ${update.check.newVersion}`
              : 'Check for updates',
          run: () => {
            // Always opens the manager: an install started from a menu with no
            // window to report it is an install that appears to do nothing.
            onOpenUpdates();
            if (update.check?.state !== 'available') void updateStore.check();
          },
        },
      ],
    },
  ];

  /**
   * An action Monaco already provides, reached through Monaco.
   *
   * Undo, find and the rest are the editor's own commands. Triggering them
   * rather than reimplementing them is what makes the menu entry behave
   * identically to the shortcut — including the undo stack being per file.
   */
  function editorAction(id: string, label: string, keybinding: string, action: string): MenuEntry {
    return {
      id,
      label,
      keybinding,
      enabled: buffer !== null,
      run: () => {
        const [instance] = monaco.editor.getEditors();
        if (!instance) return;
        instance.focus();
        instance.trigger('menu', action, null);
      },
    };
  }

  function navigate(direction: -1 | 1) {
    setHistory((current) => {
      const index = current.index + direction;
      const path = current.paths[index];
      if (path === undefined) return current;
      if (bufferFor(editor, path)) {
        setEditor((state) => ({ ...state, active: path }));
        setSelection(selectOnly(path));
      } else {
        void openPath(path);
      }
      return { ...current, index };
    });
  }

  // ----------------------------------------------------------- context menus
  function fileMenu(entry: FileEntry, event: React.MouseEvent) {
    event.preventDefault();
    event.stopPropagation();
    // A right-click inside a multi-selection keeps it, so "Delete" on the menu
    // acts on everything the user can see is selected.
    const acting = selectionForContextMenu(selection, entry.path);
    setSelection(acting);
    if (entry.kind === 'directory') setTargetDirectory(entry.path);

    const directory = entry.kind === 'directory' ? entry.path : parentOf(entry.path);
    setMenu({
      position: { x: event.clientX, y: event.clientY },
      entries: [
        {
          id: 'new-file',
          label: 'New File…',
          icon: 'new-file',
          run: () => beginCreate(directory, false),
        },
        {
          id: 'new-folder',
          label: 'New Folder…',
          icon: 'new-folder',
          run: () => beginCreate(directory, true),
        },
        { id: 'sep1', separator: true },
        {
          id: 'reveal',
          label: 'Reveal in File Explorer',
          icon: 'external',
          run: () => void reveal_(entry.path),
        },
        {
          id: 'copy-path',
          label: 'Copy Path',
          icon: 'copy',
          run: () => copyToClipboard(absolutePath(projectRoot, entry.path), 'the path'),
        },
        {
          id: 'copy-relative',
          label: 'Copy Relative Path',
          run: () => copyToClipboard(entry.path, 'the relative path'),
        },
        { id: 'sep2', separator: true },
        {
          id: 'rename',
          label: 'Rename…',
          icon: 'pencil',
          keybinding: 'F2',
          run: () => beginRename(entry),
        },
        {
          id: 'duplicate',
          label: 'Duplicate',
          icon: 'copy',
          run: () => void duplicate(entry),
        },
        { id: 'sep3', separator: true },
        {
          id: 'cut',
          label: acting.selected.length > 1 ? `Cut ${acting.selected.length} items` : 'Cut',
          keybinding: 'Ctrl+X',
          run: () => cutOrCopy('cut'),
        },
        {
          id: 'copy',
          label: acting.selected.length > 1 ? `Copy ${acting.selected.length} items` : 'Copy',
          icon: 'copy',
          keybinding: 'Ctrl+C',
          run: () => cutOrCopy('copy'),
        },
        {
          id: 'paste',
          label: 'Paste',
          keybinding: 'Ctrl+V',
          enabled: clipboard !== null,
          run: () => void paste(),
        },
        { id: 'sep4', separator: true },
        {
          id: 'delete',
          label: acting.selected.length > 1 ? `Delete ${acting.selected.length} items` : 'Delete',
          icon: 'trash',
          keybinding: 'Delete',
          danger: true,
          run: () => removePaths(acting.selected),
        },
      ],
    });
  }

  function rootMenu(event: React.MouseEvent) {
    event.preventDefault();
    setSelection(clearSelection());
    setTargetDirectory('');
    setMenu({
      position: { x: event.clientX, y: event.clientY },
      entries: [
        {
          id: 'new-file',
          label: 'New File…',
          icon: 'new-file',
          run: () => beginCreate('', false),
        },
        {
          id: 'new-folder',
          label: 'New Folder…',
          icon: 'new-folder',
          run: () => beginCreate('', true),
        },
        { id: 'sep', separator: true },
        {
          id: 'paste',
          label: 'Paste',
          keybinding: 'Ctrl+V',
          enabled: clipboard !== null,
          run: () => void paste(),
        },
        { id: 'sep2', separator: true },
        {
          id: 'refresh',
          label: 'Refresh Explorer',
          icon: 'refresh',
          run: () => runCommand('explorer.refresh'),
        },
        {
          id: 'reveal',
          label: 'Reveal in File Explorer',
          icon: 'external',
          run: () => void reveal_(''),
        },
      ],
    });
  }

  function tabMenu(path: string, event: React.MouseEvent) {
    event.preventDefault();
    const index = editor.buffers.findIndex((item) => item.path === path);
    setMenu({
      position: { x: event.clientX, y: event.clientY },
      entries: [
        { id: 'close', label: 'Close', keybinding: 'Ctrl+W', run: () => requestClose(path) },
        {
          id: 'close-others',
          label: 'Close Others',
          enabled: editor.buffers.length > 1,
          run: () => setEditor((current) => closeOthers(current, path)),
        },
        {
          id: 'close-right',
          label: 'Close to the Right',
          enabled: index >= 0 && index < editor.buffers.length - 1,
          run: () => setEditor((current) => closeToRight(current, path)),
        },
        { id: 'close-all', label: 'Close All', run: () => setEditor(closeAll()) },
        { id: 'sep', separator: true },
        {
          id: 'copy-path',
          label: 'Copy Path',
          icon: 'copy',
          run: () => copyToClipboard(absolutePath(projectRoot, path), 'the path'),
        },
        {
          id: 'reveal',
          label: 'Reveal in File Explorer',
          icon: 'external',
          run: () => void reveal_(path),
        },
        {
          id: 'reveal-explorer',
          label: 'Reveal in Explorer View',
          run: () => {
            showSidebar('explorer');
            setSelection(selectOnly(path));
            const directory = parentOf(path);
            if (directory) {
              setExpanded((current) =>
                current.includes(directory) ? current : [...current, directory],
              );
              void loadDirectory(directory);
            }
          },
        },
      ],
    });
  }

  // ------------------------------------------------------------------ render
  const treeState: TreeState = {
    listings,
    expanded,
    selection,
    targetDirectory,
    editing,
  };

  const welcomeActions: WelcomeAction[] = [
    {
      id: 'new',
      label: 'New File…',
      icon: 'new-file',
      keybinding: 'Ctrl+N',
      run: () => runCommand('file.new'),
    },
    {
      id: 'open',
      label: 'Go to File…',
      icon: 'search',
      keybinding: 'Ctrl+P',
      run: () => setPalette('files'),
    },
    {
      id: 'explorer',
      label: 'Show Explorer',
      icon: 'file',
      keybinding: 'Ctrl+Shift+E',
      run: () => runCommand('view.explorer'),
    },
    {
      id: 'terminal',
      label: 'Open Terminal Panel',
      icon: 'terminal',
      keybinding: 'Ctrl+`',
      run: () => showPanel('terminal'),
    },
    {
      id: 'run',
      label: 'Start Project',
      icon: 'play',
      enabled: project.status !== 'RUNNING' && !false && busyAction === null,
      run: () => runCommand('project.start'),
    },
  ];

  return (
    <div className="vs-root flex h-full min-h-0 flex-col bg-vs-editor">
      <TitleBar
        menus={menus}
        projectName={project.displayName}
        canGoBack={history.index > 0}
        canGoForward={history.index < history.paths.length - 1}
        onBack={() => navigate(-1)}
        onForward={() => navigate(1)}
        onOpenPalette={() => setPalette('files')}
        update={
          update.check?.state === 'available'
            ? { label: `Update ${update.check.newVersion}`, onOpen: onOpenUpdates }
            : null
        }
        sidebarVisible={layout.sidebarVisible}
        panelVisible={layout.panelVisible}
        onToggleSidebar={() => runCommand('view.toggleSidebar')}
        onTogglePanel={() => runCommand('view.togglePanel')}
      />

      <div className="flex min-h-0 flex-1">
        <ActivityBar
          view={layout.activityView}
          visible={layout.sidebarVisible}
          unsaved={dirtyCount}
          onSelect={(view) =>
            patchLayout(
              layout.activityView === view && layout.sidebarVisible
                ? { sidebarVisible: false }
                : { sidebarVisible: true, activityView: view },
            )
          }
          onProjects={leave}
          onSettings={onOpenSettings}
        />

        {layout.sidebarVisible && (
          <>
            <aside
              style={{ width: `${layout.sidebarWidth}px` }}
              className="flex min-h-0 shrink-0 flex-col bg-vs-sidebar"
            >
              {layout.activityView === 'explorer' && (
                <ExplorerPanel
                  ref={explorer}
                  projectName={project.displayName}
                  state={treeState}
                  callbacks={{
                    onOpen: (entry) => void openPath(entry.path),
                    onToggle: (path) => {
                      setExpanded((current) => toggleExpanded(current, path));
                      if (!listings[path]) void loadDirectory(path);
                    },
                    onContextMenu: fileMenu,
                    onMove: (paths, to) => void movePaths(paths, to),
                    dragPathsFor: (path) => dragPathsFor(selection, path),
                    onRowPointerDown: (entry, event) =>
                      setSelection((current) =>
                        selectFromPointer(current, entry.path, visibleEntries, {
                          ctrl: event.ctrlKey || event.metaKey,
                          shift: event.shiftKey,
                        }),
                      ),
                    onSelectDirectory: (path) => setTargetDirectory(path),
                  }}
                  actions={{
                    onNewFile: () => beginCreate(targetDirectory, false),
                    onNewFolder: () => beginCreate(targetDirectory, true),
                    onRefresh: () => runCommand('explorer.refresh'),
                    onCollapseAll: () => setExpanded([]),
                  }}
                  uploads={uploads}
                  draggingOver={draggingOver}
                  dropTargetLabel={displayDirectory(targetDirectory)}
                  onSubmitEdit={(value) => void submitEdit(value)}
                  onCancelEdit={() => setEditing(null)}
                  onCancelUpload={cancelUpload}
                  onRetryUpload={retryUpload}
                  onClearFinishedUploads={() =>
                    setUploads((current) =>
                      current.filter(
                        (item) => item.status === 'queued' || item.status === 'uploading',
                      ),
                    )
                  }
                  onEmptyAreaClick={() => {
                    setSelection(clearSelection());
                    setTargetDirectory('');
                  }}
                  onEmptyAreaContextMenu={rootMenu}
                  onRubberBandSelect={(paths) => setSelection(selectPaths(paths))}
                  onDragEnter={onDragEnter}
                  onDragLeave={onDragLeave}
                  onDragOver={onDragOver}
                  onDrop={onDrop}
                />
              )}

              {layout.activityView === 'search' && (
                <SearchPanel
                  query={searchQuery}
                  onQueryChange={setSearchQuery}
                  results={sidebarSearch.results}
                  searching={sidebarSearch.searching}
                  failure={sidebarSearch.failure}
                  onOpen={(entry) => {
                    if (entry.kind === 'directory') {
                      setTargetDirectory(entry.path);
                      showSidebar('explorer');
                      setExpanded((current) =>
                        current.includes(entry.path) ? current : [...current, entry.path],
                      );
                      void loadDirectory(entry.path);
                      return;
                    }
                    void openPath(entry.path);
                  }}
                />
              )}

              {layout.activityView === 'source-control' && (
                <SourceControlPanel projectRoot={projectRoot} />
              )}
              {layout.activityView === 'run' && (
                <RunPanel project={project} onOpenTerminal={() => showPanel('terminal')} />
              )}
              {layout.activityView === 'extensions' && <ExtensionsPanel />}
              {layout.activityView === 'account' && (
                <AccountPanel status={status} openFiles={editor.buffers.map((item) => item.path)} />
              )}
            </aside>

            <ResizeHandle
              orientation="vertical"
              label="Resize the side bar"
              value={layout.sidebarWidth}
              onResize={(next, source) =>
                patchLayout({
                  sidebarWidth: clampSidebarWidth(
                    // A pointer reports where it is; the keyboard reports a
                    // size. The activity bar's 48px is the difference.
                    source === 'pointer' ? next - 48 : next,
                    window.innerWidth,
                  ),
                })
              }
              onDoubleClick={() => patchLayout({ sidebarVisible: false })}
            />
          </>
        )}

        <main className="flex min-w-0 flex-1 flex-col">
          <div className="flex min-h-0 flex-1 flex-col">
            {editor.buffers.length > 0 && (
              <EditorTabs
                buffers={editor.buffers}
                active={editor.active}
                onSelect={(path) => {
                  setEditor((current) => ({ ...current, active: path }));
                  setSelection(selectOnly(path));
                }}
                onClose={requestClose}
                onContextMenu={tabMenu}
              />
            )}

            {buffer?.readOnly && (
              <p className="flex shrink-0 items-center gap-1.5 border-b border-amber-900/60 bg-amber-950/40 px-3 py-1 text-[12px] text-amber-300">
                <Icon name="warning" size={13} />
                This project is being built or removed, so its files are read-only right now.
              </p>
            )}

            <div className="min-h-0 flex-1">
              {buffer ? (
                <CodeEditor
                  path={buffer.path}
                  language={buffer.language}
                  value={buffer.current}
                  readOnly={buffer.readOnly}
                  onChange={(text) => setEditor((current) => edit(current, buffer.path, text))}
                  onSave={() => void save(buffer.path)}
                  onCursor={setCursor}
                  reveal={reveal}
                />
              ) : (
                <EmptyEditorState projectName={project.displayName} actions={welcomeActions} />
              )}
            </div>
          </div>

          {layout.panelVisible && (
            <>
              <ResizeHandle
                orientation="horizontal"
                label="Resize the panel"
                value={layout.panelHeight}
                onResize={(next, source) =>
                  patchLayout({
                    panelHeight: clampPanelHeight(
                      source === 'pointer' ? window.innerHeight - next - 22 : next,
                      window.innerHeight,
                    ),
                  })
                }
                onDoubleClick={() => patchLayout({ panelVisible: false })}
              />
              <div style={{ height: `${layout.panelHeight}px` }} className="flex min-h-0 shrink-0">
                <BottomPanel
                  tab={layout.panelTab}
                  onSelect={(tab) => patchLayout({ panelTab: tab })}
                  onClose={() => patchLayout({ panelVisible: false })}
                  problemCount={errors + warnings}
                  children={{
                    problems: (
                      <ProblemsPanel
                        problems={problems}
                        hasOpenFiles={editor.buffers.length > 0}
                        onSelect={(problem: Problem) =>
                          goTo(problem.path, problem.line, problem.column)
                        }
                      />
                    ),
                    output: (
                      <OutputPanel
                        lines={linesForChannel(output, outputChannel)}
                        channel={outputChannel}
                        onSelectChannel={setOutputChannel}
                        onClear={() => setOutput([])}
                      />
                    ),
                    terminal: (
                      <TerminalPanel
                        project={project}
                        busy={busyAction}
                        onAction={(action) => void runProject(action)}
                        lines={linesForChannel(output, 'Project')}
                      />
                    ),
                    logs: (
                      <LogsPanel
                        settings={settings}
                        onCopyPath={(path) => copyToClipboard(path, 'the path')}
                      />
                    ),
                  }}
                />
              </div>
            </>
          )}
        </main>
      </div>

      <StatusBar
        projectStatus={project.status}
        running={project.status === 'RUNNING'}
        errors={errors}
        warnings={warnings}
        language={buffer ? languageName(buffer.language) : null}
        cursor={buffer ? cursor : null}
        lineEnding={buffer ? (buffer.current.includes('\r\n') ? 'CRLF' : 'LF') : null}
        dirty={buffer !== null && isDirty(buffer)}
        saving={saving}
        onProblems={() => showPanel('problems')}
        onRun={() => showPanel('terminal')}
      />

      {palette && (
        <CommandPalette
          mode={palette}
          commands={commands}
          paths={quickOpenPaths}
          loadingPaths={quickOpen.searching}
          onFileQuery={setQuickOpenTerm}
          onRunCommand={(command) => command.run()}
          onOpenPath={(path) => void openPath(path)}
          onClose={() => setPalette(null)}
        />
      )}

      {menu && (
        <ContextMenu
          entries={menu.entries}
          position={menu.position}
          onClose={() => setMenu(null)}
        />
      )}

      {operationProgress && (
        <ImportProgressDialog
          operation={operationProgress}
          onCancel={() => {
            cancelledOperations.current.add(operationProgress.operationId);
            // The core keys cancellation by *import* id, so every batch this
            // operation has in flight is named individually. Sending the
            // operation id would cancel nothing at all.
            for (const [importId, inFlight] of activeImports.current) {
              if (inFlight.operationId !== operationProgress.operationId) continue;
              void cancelProjectFileImport(importId).catch(() => undefined);
            }
            setOperationProgress((current) =>
              current === null ? null : requestCancellation(current),
            );
          }}
          onClose={() => {
            operationRef.current = null;
            setOperationProgress(null);
          }}
        />
      )}

      {organising && (
        <ImportOrganiser
          groups={organising.groups}
          candidates={organising.candidates}
          destination={organising.directory}
          existingPaths={organising.existing}
          onChange={(groups) =>
            setOrganising((current) => (current ? { ...current, groups } : null))
          }
          onCancel={() => {
            setOrganising(null);
            say('Transfers', 'Import cancelled.', 'warn');
          }}
          onImport={() => {
            const { groups, directory } = organising;
            setOrganising(null);
            void runOrganisedImport(groups, directory);
          }}
        />
      )}

      {importPreview && (
        <ImportPreviewDialog
          plan={importPreview.plan}
          targetDirectory={importPreview.directory}
          onCancel={() => {
            setImportPreview(null);
            say('Transfers', 'Import cancelled.', 'warn');
          }}
          onConfirm={(unwrap) => {
            const { plan, directory } = importPreview;
            setImportPreview(null);
            const unwrapPaths = unwrap ? plan.unwrapPaths : [];
            say(
              'Transfers',
              unwrap
                ? describePlan(plan, directory)
                : `Importing ${plan.sourcePaths.length} item${plan.sourcePaths.length === 1 ? '' : 's'} into ${displayDirectory(directory)}.`,
            );
            startNativeImport(plan.sourcePaths, unwrapPaths, directory);
          }}
        />
      )}

      {conflictReview && (
        <BatchConflictDialog
          // Remounted on every re-analysis: a dialog that kept its state would
          // be showing decisions about conflicts that no longer exist.
          key={conflictReview.revision}
          conflicts={conflictReview.conflicts}
          operation={conflictReview.operation}
          existing={conflictReview.existing}
          items={conflictReview.items}
          existingKinds={conflictReview.existingKinds}
          grouping={conflictReview.grouping}
          directories={conflictReview.directories}
          initialDecisions={conflictReview.initialDecisions}
          notice={conflictReview.notice}
          onRelocate={(request, decisions) =>
            conflictReview.resolve({ kind: 'relocate', request, decisions })
          }
          onCancel={() => conflictReview.resolve({ kind: 'cancelled' })}
          onConfirm={(decisions) => conflictReview.resolve({ kind: 'confirmed', decisions })}
        />
      )}

      {conflict && (
        <ConflictDialog
          conflicts={conflict.paths}
          targetDirectory={conflict.directory}
          onChoose={(choice) => conflict.resolve(choice)}
          onCancel={() => conflict.resolve(null)}
        />
      )}

      {confirm && (
        <ConfirmDialog
          title={confirm.title}
          detail={confirm.detail}
          confirmLabel={confirm.confirmLabel}
          danger={confirm.danger}
          onConfirm={confirm.onConfirm}
          onCancel={() => setConfirm(null)}
        />
      )}

      {gate}
    </div>
  );
}

/**
 * What the conflict dialog can come back with.
 *
 * Relocation is deliberately not a decision: it does not settle a collision, it
 * replaces the question. Modelling it as a third outcome is what stops it being
 * mistaken for one and executed.
 */
type ConflictOutcome =
  | { kind: 'confirmed'; decisions: Decisions }
  | { kind: 'relocate'; request: RelocationRequest; decisions: Decisions }
  | { kind: 'cancelled' };

/**
 * What to tell the user after a destination change.
 *
 * The number of *new* conflicts is the part worth saying out loud: moving
 * something to a folder that already has one of those is a common way to swap
 * one collision for another without noticing.
 */
function describeRelocation(plan: RelocationPlan, newConflicts: number): string {
  const parts: string[] = [];
  parts.push(
    plan.moved.length === 1
      ? 'The destination has been changed and the conflicts checked again.'
      : `${plan.moved.length} destinations have been changed and the conflicts checked again.`,
  );
  if (plan.refused.length > 0) {
    parts.push(
      `${plan.refused.length} could not be moved there: ${plan.refused[0]?.message ?? ''}`,
    );
  }
  if (newConflicts > 0) {
    parts.push(
      `That created ${newConflicts} new conflict${newConflicts === 1 ? '' : 's'}, which must be decided before continuing.`,
    );
  }
  return parts.join(' ');
}

/** The last component of a project-relative path. */
function nameOfPath(path: string): string {
  const cut = path.lastIndexOf('/');
  return cut < 0 ? path : path.slice(cut + 1);
}

/** The absolute path of a project file, for display and for the clipboard. */
function absolutePath(root: string | null, path: string): string {
  if (!root) return path;
  const separator = root.includes('\\') ? '\\' : '/';
  const tail = separator === '\\' ? path.replace(/\//g, '\\') : path;
  return path ? `${root}${separator}${tail}` : root;
}

/** The entry for a path, from whichever listing holds it. */
function findEntry(listings: Record<string, FileEntry[]>, path: string): FileEntry | null {
  for (const entries of Object.values(listings)) {
    const found = entries.find((entry) => entry.path === path);
    if (found) return found;
  }
  return null;
}

function isInternalDrag(event: React.DragEvent): boolean {
  return event.dataTransfer.types.includes('application/x-project-path');
}
