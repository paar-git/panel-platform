/**
 * The application's frame.
 *
 * One shell for the whole product: a top bar, a tool rail, a resizable sidebar,
 * a tabbed workspace and a status bar. Everything the window shows is inside
 * it, so there is one navigation system rather than a page shell and an editor
 * shell that happen to be in the same binary.
 *
 * State lives in `App`, which owns the polling and the run actions. This
 * component decides what is on screen, not what is true — which is why the
 * layout can be rearranged without touching anything that talks to the core.
 */
import { useCallback, useEffect, useState } from 'react';

import type { MachineLoad, PowerStatus, ProjectSummary, SystemStatus } from '../api';
import ProjectConsole from '../components/ProjectConsole';
import ProcessesTool from '../tools/ProcessesTool';
import ProjectsTool from '../tools/ProjectsTool';
import ResourcesTool from '../tools/ResourcesTool';
import { EnvironmentTool, PortsTool } from '../tools/InspectTools';
import { ToolBody, ToolEmpty, ToolHeader } from '../tools/ToolChrome';
import ProjectOverviewTab from '../workspace/ProjectOverviewTab';
import ResizeHandle from '../workspace/ResizeHandle';
import ActivityRail from './ActivityRail';
import ShellStatusBar from './ShellStatusBar';
import WorkspaceTabs, { type WorkspaceTab } from './WorkspaceTabs';
import WorkspaceTopBar from './WorkspaceTopBar';
import { clampSidebar, type ShellLayout, type ToolId } from './shellLayout';

/** Which pane of the workspace is showing for the open project. */
type WorkspaceTabId = 'overview' | 'console' | 'details';

export default function AppShell({
  status,
  power,
  load,
  projects,
  project,
  failure,
  busy,
  updateAvailable,
  settingsPane,
  projectsPane,
  detailsPane,
  discordPane,
  layout,
  patch,
  onOpenProject,
  onNewProject,
  onRefresh,
  onOpenPalette,
  onOpenActivity,
  onInstallUpdate,
  onOpenFiles,
  onStart,
  onStop,
  onRestart,
}: {
  status: SystemStatus | null;
  power: PowerStatus | null;
  load: MachineLoad | null;
  projects: ProjectSummary[] | null;
  project: ProjectSummary | null;
  failure: string | null;
  busy: boolean;
  updateAvailable: string | null;
  /** Rendered in the workspace when the Settings tool is chosen. */
  settingsPane: React.ReactNode;
  /** Owned by `App`, so the command palette can select a tool. */
  layout: ShellLayout;
  patch: (changes: Partial<ShellLayout>) => void;
  /** Rendered in the workspace when no project is open. */
  projectsPane: React.ReactNode;
  /** The open project's full record: deployments, history, limits, power. */
  detailsPane: React.ReactNode;
  /** Discord bot management, which is a real feature and needs a way in. */
  discordPane: React.ReactNode;
  onOpenProject: (id: string) => void;
  onNewProject: () => void;
  onRefresh: () => void;
  onOpenPalette: () => void;
  onOpenActivity: () => void;
  onInstallUpdate: () => void;
  onOpenFiles: () => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
}) {
  const [tab, setTab] = useState<WorkspaceTabId>('overview');

  /**
   * Choosing the tool that is already showing collapses the sidebar, and
   * choosing it again brings it back.
   */
  const selectTool = useCallback(
    (next: ToolId) => {
      if (next === 'console') {
        // Console is a workspace pane rather than a navigator: the rail entry
        // brings it to the front instead of replacing the sidebar with a
        // second copy of it.
        setTab('console');
        return;
      }
      if (next === layout.tool && layout.sidebarVisible) {
        patch({ sidebarVisible: false });
        return;
      }
      patch({ tool: next, sidebarVisible: true });
    },
    [layout.sidebarVisible, layout.tool, patch],
  );

  // Ctrl+B toggles the sidebar, matching every editor anyone arrives from.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key.toLowerCase() !== 'b') return;
      event.preventDefault();
      patch({ sidebarVisible: !layout.sidebarVisible });
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [layout.sidebarVisible, patch]);

  const runningCount = (projects ?? []).filter((entry) => entry.status === 'RUNNING').length;
  const showSettings = layout.tool === 'settings';
  const showDiscord = layout.tool === 'discord';

  const tabs: WorkspaceTab[] =
    project === null || showSettings || showDiscord
      ? []
      : [
          { id: 'overview', label: 'Overview', icon: 'overview' },
          {
            id: 'console',
            label: 'Console',
            icon: 'terminal',
            dot: project.status === 'RUNNING',
          },
          { id: 'details', label: 'Details', icon: 'list' },
        ];

  return (
    <div className="flex h-full min-h-0 flex-col bg-canvas text-ink">
      <WorkspaceTopBar
        status={status}
        projects={projects}
        project={project}
        busy={busy}
        updateAvailable={updateAvailable}
        onOpenPalette={onOpenPalette}
        onOpenProject={onOpenProject}
        onBrowseProjects={() => patch({ tool: 'projects', sidebarVisible: true })}
        onStart={onStart}
        onStop={onStop}
        onRestart={onRestart}
        onOpenConsole={() => setTab('console')}
        onConfigureRun={onOpenFiles}
        onOpenActivity={onOpenActivity}
        onInstallUpdate={onInstallUpdate}
      />

      <div className="flex min-h-0 flex-1">
        <ActivityRail
          tool={layout.tool}
          visible={layout.sidebarVisible}
          runningCount={runningCount}
          onSelect={selectTool}
        />

        {layout.sidebarVisible && (
          <>
            <aside
              className="flex min-h-0 shrink-0 flex-col border-r border-edge bg-surface"
              style={{ width: `${layout.sidebarWidth}px` }}
            >
              <ToolPanel
                tool={layout.tool}
                projects={projects}
                project={project}
                power={power}
                onOpenProject={onOpenProject}
                onNewProject={onNewProject}
                onRefresh={onRefresh}
                onBrowseProjects={() => patch({ tool: 'projects' })}
              />
            </aside>

            <ResizeHandle
              orientation="vertical"
              label="Resize the sidebar"
              value={layout.sidebarWidth}
              onResize={(next, source) => {
                // A pointer drag reports where the pointer is; the keyboard
                // reports the size it wants. The rail sits to the left of the
                // sidebar, so a pointer position has to lose its width.
                const raw = source === 'pointer' ? next - 48 : next;
                patch({ sidebarWidth: clampSidebar(raw, window.innerWidth) });
              }}
              onDoubleClick={() => patch({ sidebarWidth: 248 })}
            />
          </>
        )}

        <main className="flex min-w-0 flex-1 flex-col bg-canvas">
          {failure !== null && (
            <div className="shrink-0 border-b border-danger/30 bg-danger-soft px-3 py-1.5 text-[12px] text-danger">
              {failure}
            </div>
          )}

          {showSettings ? (
            <div className="min-h-0 flex-1 overflow-y-auto">{settingsPane}</div>
          ) : showDiscord ? (
            <div className="min-h-0 flex-1 overflow-y-auto">{discordPane}</div>
          ) : project === null ? (
            <div className="min-h-0 flex-1 overflow-y-auto">{projectsPane}</div>
          ) : (
            <>
              <WorkspaceTabs
                tabs={tabs}
                active={tab}
                onSelect={(id) => setTab(id as WorkspaceTabId)}
              />
              {tab === 'console' ? (
                <div className="min-h-0 flex-1">
                  <ProjectConsole projectId={project.id} fill />
                </div>
              ) : tab === 'details' ? (
                <div className="min-h-0 flex-1 overflow-y-auto">{detailsPane}</div>
              ) : (
                <ProjectOverviewTab
                  project={project}
                  busy={busy}
                  onOpenConsole={() => setTab('console')}
                  onOpenFiles={onOpenFiles}
                  onRestart={onRestart}
                />
              )}
            </>
          )}
        </main>
      </div>

      <ShellStatusBar
        runningCount={runningCount}
        load={load}
        power={power}
        onOpenProcesses={() => patch({ tool: 'processes', sidebarVisible: true })}
        onOpenResources={() => patch({ tool: 'resources', sidebarVisible: true })}
      />
    </div>
  );
}

/** Which panel the rail's selection puts in the sidebar. */
function ToolPanel({
  tool,
  projects,
  project,
  power,
  onOpenProject,
  onNewProject,
  onRefresh,
  onBrowseProjects,
}: {
  tool: ToolId;
  projects: ProjectSummary[] | null;
  project: ProjectSummary | null;
  power: PowerStatus | null;
  onOpenProject: (id: string) => void;
  onNewProject: () => void;
  onRefresh: () => void;
  onBrowseProjects: () => void;
}) {
  switch (tool) {
    case 'projects':
      return (
        <ProjectsTool
          projects={projects}
          currentId={project?.id ?? null}
          onOpen={onOpenProject}
          onNewProject={onNewProject}
          onRefresh={onRefresh}
        />
      );
    case 'processes':
      return <ProcessesTool currentId={project?.id ?? null} onOpen={onOpenProject} />;
    case 'ports':
      return <PortsTool project={project} onOpenProjects={onBrowseProjects} />;
    case 'environment':
      return <EnvironmentTool project={project} onOpenProjects={onBrowseProjects} />;
    case 'resources':
      return <ResourcesTool power={power} />;
    case 'discord':
      return (
        <>
          <ToolHeader title="Discord" />
          <ToolBody>
            <ToolEmpty message="Bots are open in the workspace." />
          </ToolBody>
        </>
      );
    case 'settings':
      return (
        <>
          <ToolHeader title="Settings" />
          <ToolBody>
            <ToolEmpty message="Settings are open in the workspace." />
          </ToolBody>
        </>
      );
    default:
      return null;
  }
}
