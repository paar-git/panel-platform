/**
 * Settings.
 *
 * Grouped into tabs, with real controls where a control does something and a
 * plain reading where it does not. The core has no command that writes its
 * configuration — it is loaded from `config.toml` and the environment at
 * startup — so most of this screen reports rather than edits.
 *
 * Rather than dress that up with switches that forget what you set, the values
 * that cannot change are read-only rows, the ones that can (this window's own
 * preferences, the update check, opening a folder) are real controls, and the
 * screen says which is which once, at the top of General.
 */
import { useEffect, useState } from 'react';

import {
  appSettings,
  errorMessage,
  powerJournal,
  revealProjectPath,
  setPowerMode,
  startupDiagnostics,
  systemMetrics,
  type AppSettings,
  type PowerJournalEntry,
  type PowerMode,
  type PowerStatus,
  type ProjectSummary,
  type StartupDiagnostics,
  type SystemMetrics,
  type SystemStatus,
} from '../api';
import { formatBytes, formatDuration } from '../lib/format';
import { batteryPhrase, powerLook, temperaturePhrase } from '../lib/power';
import { installBusy } from '../update';
import { useUpdate } from '../useUpdate';
import Icon from '../ui/Icon';
import { ACCENTS, type Appearance, type Density, type MotionLevel } from '../lib/appearance';
import ThemeBrowser from '../components/ThemeBrowser';
import { ConfirmDialog } from '../ui/overlays';
import Select from '../ui/Select';
import {
  Badge,
  Button,
  Card,
  CardHeader,
  DataRow,
  PageShell,
  Skeleton,
  Tabs,
  Toggle,
} from '../ui/primitives';
import { toast } from '../ui/toast';

type TabId =
  | 'general'
  | 'appearance'
  | 'preferences'
  | 'advanced'
  | 'power'
  | 'storage'
  | 'networking'
  | 'updates'
  | 'logging'
  | 'about';

/** Where the application opens. `last` restores whatever was on screen. */
export type StartupView = 'overview' | 'projects' | 'activity' | 'last';

/** This window's own preferences — the settings that genuinely are writable. */
export interface Preferences {
  collapsedSidebar: boolean;
  confirmDestructive: boolean;
  appearance: Appearance;
  startupView: StartupView;
  /** Show a toast when a project changes state on its own. */
  notifyStateChanges: boolean;
  /** Reveals internal ids, raw errors and timing in the interface. */
  developerMode: boolean;
}

/**
 * The five modes, in the order they are offered.
 *
 * Automatic first because it is the recommended one, manual last because it is
 * the one that asks the application to stop deciding.
 */
const POWER_MODES: { id: PowerMode; label: string; description: string }[] = [
  {
    id: 'automatic',
    label: 'Automatic',
    description: 'Decide from what the machine is doing. Recommended.',
  },
  {
    id: 'performance',
    label: 'Performance',
    description: 'Always prefer your projects being responsive.',
  },
  {
    id: 'balanced',
    label: 'Balanced',
    description: 'Stay in the middle whatever the machine is doing.',
  },
  {
    id: 'efficiency',
    label: 'Efficiency',
    description: 'Always prefer using less. No project is ever stopped for this.',
  },
  {
    id: 'manual',
    label: 'Manual',
    description:
      'Watch and report, and change nothing automatically. Sleep is still held off for a project that asked.',
  },
];

export default function Settings({
  status,
  power,
  projects,
  preferences,
  onPreferences,
  onResetLayout,
  onOpenUpdates,
}: {
  status: SystemStatus | null;
  power: PowerStatus | null;
  projects: ProjectSummary[] | null;
  preferences: Preferences;
  onPreferences: (next: Partial<Preferences>) => void;
  onResetLayout: () => void;
  /** Opens the update manager and asks the feed. */
  onOpenUpdates: () => void;
}) {
  const [tab, setTab] = useState<TabId>('general');
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [metrics, setMetrics] = useState<SystemMetrics | null>(null);
  const [confirmReset, setConfirmReset] = useState(false);
  const [journal, setJournal] = useState<PowerJournalEntry[]>([]);
  const [launch, setLaunch] = useState<StartupDiagnostics | null>(null);
  const update = useUpdate();

  const powerSummary = powerLook(power);
  const battery = batteryPhrase(power);
  const temperature = temperaturePhrase(power);

  // Read once when the tab is opened, and then on the shell's poll. Asked for
  // from zero every time rather than with a cursor: this is the last few
  // entries for a person to read, not a live feed, and a cursor kept across
  // tab switches would show an empty list to somebody returning to the page.
  useEffect(() => {
    if (tab !== 'power') return undefined;

    let cancelled = false;
    const read = () => {
      powerJournal(0)
        .then((page) => {
          if (!cancelled) setJournal(page.entries.slice(-20).reverse());
        })
        .catch(() => {
          // A journal that cannot be read is not worth interrupting anyone
          // over; the rest of the panel still reports the live state.
        });
    };

    read();
    const timer = setInterval(read, 5000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [tab]);
  const checking = update.checking;

  useEffect(() => {
    appSettings()
      .then(setSettings)
      .catch((error: unknown) => toast.error('Could not read the settings', errorMessage(error)));
    systemMetrics()
      .then(setMetrics)
      .catch(() => setMetrics(null));
  }, []);

  useEffect(() => {
    if (tab !== 'logging') return;
    startupDiagnostics()
      .then(setLaunch)
      .catch(() => setLaunch(null));
  }, [tab]);

  function copy(value: string, what: string) {
    navigator.clipboard
      .writeText(value)
      .then(() => toast.success(`${what} copied`))
      .catch((error: unknown) => toast.error('Could not copy', errorMessage(error)));
  }

  /**
   * Opening a folder goes through the project reveal command, which resolves
   * inside a project root. Only a project's own folder can be opened this way,
   * so the data directory offers a copy button instead of a broken one.
   */
  function openProjectFolder() {
    const first = projects?.[0];
    if (!first) {
      toast.info('No project to open', 'Create a project first.');
      return;
    }
    revealProjectPath(first.id, '').catch((error: unknown) =>
      toast.error('Could not open the folder', errorMessage(error)),
    );
  }

  const pathRow = (label: string, value: string | undefined) => (
    <div className="flex items-center gap-2 border-b border-edge/60 py-2 last:border-b-0">
      <span className="shrink-0 text-[13px] text-muted">{label}</span>
      <span className="min-w-0 flex-1 truncate text-right font-mono text-[12px] text-ink select-text">
        {value ?? '—'}
      </span>
      <Button
        size="sm"
        variant="ghost"
        icon="copy"
        disabled={!value}
        title={`Copy the ${label.toLowerCase()}`}
        onClick={() => value && copy(value, label)}
      />
    </div>
  );

  return (
    <PageShell title="Settings" description="What this installation is running, and where.">
      <Tabs<TabId>
        active={tab}
        onSelect={setTab}
        tabs={[
          { id: 'general', label: 'General' },
          { id: 'appearance', label: 'Appearance' },
          { id: 'preferences', label: 'Preferences' },
          { id: 'advanced', label: 'Advanced' },
          { id: 'power', label: 'Power' },
          { id: 'storage', label: 'Storage' },
          { id: 'networking', label: 'Networking' },
          { id: 'updates', label: 'Updates' },
          { id: 'logging', label: 'Logging' },
          { id: 'about', label: 'System information' },
        ]}
      />

      <div className="pt-4">
        {tab === 'general' && (
          <div className="grid gap-4 lg:grid-cols-2">
            <Card>
              <CardHeader title="This window" subtitle="Preferences stored on this machine" />
              <div className="py-1">
                <Toggle
                  checked={preferences.collapsedSidebar}
                  onChange={(collapsedSidebar) => onPreferences({ collapsedSidebar })}
                  label="Collapse the sidebar"
                  description="Show icons only. Ctrl+B toggles it at any time."
                />
                <Toggle
                  checked={preferences.confirmDestructive}
                  onChange={(confirmDestructive) => onPreferences({ confirmDestructive })}
                  label="Confirm destructive actions"
                  description="Ask before force-killing a project or deleting a file."
                />
              </div>
              <div className="border-t border-edge px-4 py-3">
                <Button icon="restart" onClick={() => setConfirmReset(true)}>
                  Reset layout and preferences
                </Button>
              </div>
            </Card>

            <Card>
              <CardHeader title="Configuration" />
              <div className="px-4 py-3">
                <p className="text-[13px] leading-relaxed text-muted">
                  Everything outside this card is read from{' '}
                  <code className="rounded-[4px] bg-canvas px-1 py-0.5 font-mono text-[12px] text-ink select-text">
                    config.toml
                  </code>{' '}
                  and{' '}
                  <code className="rounded-[4px] bg-canvas px-1 py-0.5 font-mono text-[12px] text-ink select-text">
                    PROJECT_HOST_*
                  </code>{' '}
                  when the application starts. The core has no command that writes them back, so
                  those screens report rather than edit — change the file and restart.
                </p>
              </div>
              <div className="border-t border-edge py-1">
                <DataRow label="Mode" value={settings?.mode ?? '—'} />
                <DataRow label="Maximum projects" value={settings ? settings.maxProjects : '—'} />
                <DataRow
                  label="Largest upload"
                  value={settings ? formatBytes(settings.maxUploadBytes) : '—'}
                />
              </div>
            </Card>
          </div>
        )}

        {tab === 'appearance' && (
          <AppearancePanel
            appearance={preferences.appearance}
            onChange={(next) =>
              onPreferences({ appearance: { ...preferences.appearance, ...next } })
            }
          />
        )}

        {tab === 'preferences' && (
          <div className="animate-view grid gap-4 lg:grid-cols-2">
            <Card>
              <CardHeader title="On startup" subtitle="Where the window opens" />
              <div className="px-4 pb-4">
                <Select<StartupView>
                  value={preferences.startupView}
                  onChange={(startupView) => onPreferences({ startupView })}
                  options={[
                    { value: 'last', label: 'Where I left off' },
                    { value: 'overview', label: 'Overview' },
                    { value: 'projects', label: 'Projects' },
                    { value: 'activity', label: 'Activity' },
                  ]}
                />
              </div>
            </Card>

            <Card>
              <CardHeader title="Behaviour" />
              <div className="py-1">
                <Toggle
                  checked={preferences.confirmDestructive}
                  onChange={(confirmDestructive) => onPreferences({ confirmDestructive })}
                  label="Confirm destructive actions"
                  description="Ask before deleting a project or force-killing a container."
                />
                <Toggle
                  checked={preferences.collapsedSidebar}
                  onChange={(collapsedSidebar) => onPreferences({ collapsedSidebar })}
                  label="Start with the sidebar collapsed"
                  description="The rail shows icons only until you expand it."
                />
                <Toggle
                  checked={preferences.notifyStateChanges}
                  onChange={(notifyStateChanges) => onPreferences({ notifyStateChanges })}
                  label="Notify when a project changes state"
                  description="A toast when something starts, stops or fails on its own."
                />
              </div>
            </Card>
          </div>
        )}

        {tab === 'advanced' && (
          <div className="animate-view grid gap-4 lg:grid-cols-2">
            <Card>
              <CardHeader
                title="Developer mode"
                subtitle="For diagnosing this application, not your projects"
              />
              <div className="py-1">
                <Toggle
                  checked={preferences.developerMode}
                  onChange={(developerMode) => onPreferences({ developerMode })}
                  label="Show internal detail"
                  description="Project and container ids, raw error text, and how long each call took."
                />
              </div>
            </Card>

            <Card>
              <CardHeader title="Reset" subtitle="Only this window's settings — never a project" />
              <div className="flex flex-col gap-2 px-4 pb-4">
                <p className="text-[13px] text-muted">
                  Returns the theme, layout and behaviour on this machine to their defaults.
                  Projects, files, containers and credentials are untouched.
                </p>
                <div>
                  <Button icon="refresh" onClick={() => setConfirmReset(true)}>
                    Reset this window
                  </Button>
                </div>
              </div>
            </Card>
          </div>
        )}

        {tab === 'power' && (
          <div className="grid gap-4 lg:grid-cols-2">
            <Card>
              <CardHeader title="How power is managed" />
              <div className="px-4 pb-3 pt-2">
                <p className="text-[12px] text-muted">
                  Two things are changed and nothing else: whether this machine is allowed to sleep
                  on its own, and how the operating system schedules your projects against
                  everything else. Neither can stop a project, and no power plan or overlay is
                  touched — a control that silently did nothing on a machine with a custom plan
                  would be worse than not having one.
                </p>

                <div className="mt-3 grid gap-1.5">
                  {POWER_MODES.map((option) => (
                    <label
                      key={option.id}
                      className={`flex cursor-pointer items-start gap-2.5 rounded-[8px] border px-3 py-2.5 ${
                        power?.mode === option.id
                          ? 'border-accent/50 bg-accent-soft'
                          : 'border-edge hover:bg-raised'
                      }`}
                    >
                      <input
                        type="radio"
                        name="power-mode"
                        className="mt-1"
                        checked={power?.mode === option.id}
                        onChange={() => {
                          void setPowerMode(option.id)
                            .then(() => {
                              toast.info(`Power mode set to ${option.label.toLowerCase()}`);
                            })
                            .catch((error: unknown) => {
                              toast.error('Could not change the power mode', errorMessage(error));
                            });
                        }}
                      />
                      <span className="min-w-0">
                        <span className="block text-[13px] text-ink">{option.label}</span>
                        <span className="block text-[12px] text-muted">{option.description}</span>
                      </span>
                    </label>
                  ))}
                </div>
              </div>
            </Card>

            <Card>
              <CardHeader title="What this machine is doing" />
              <div className="py-1">
                <DataRow
                  label="In force"
                  value={
                    power !== null && power.measured ? (
                      <Badge tone={powerSummary.tone === 'warn' ? 'warn' : 'neutral'} dot>
                        {powerSummary.label}
                      </Badge>
                    ) : (
                      'measuring…'
                    )
                  }
                />
                <DataRow
                  label="Processor"
                  value={
                    power?.cpuPercent === null || power === null
                      ? '—'
                      : `${Math.round(power.cpuPercent)}%`
                  }
                />
                {/* Absent rather than zero. Most Windows desktops expose no
                    readable sensor, and 0°C would be an invented reading. */}
                <DataRow label="Temperature" value={temperature ?? 'no readable sensor'} />
                <DataRow label="Battery" value={battery ?? 'no battery'} />
                <DataRow
                  label="Holding sleep off"
                  value={power?.sleepHeld === true ? 'yes' : 'no'}
                />
                <DataRow label="Projects running" value={String(power?.activeProjects ?? 0)} />
              </div>

              {power !== null && power.measured && (
                <div className="mx-4 mb-4 rounded-[8px] border border-edge bg-raised px-3 py-2.5">
                  {/* The sentence the policy engine produced, verbatim. A
                      profile shown without its reason explains nothing. */}
                  <p className="text-[12px] text-muted">{power.reason}</p>
                </div>
              )}

              {power?.preventSleep === true && !power.sleepHeld && (
                <div className="mx-4 mb-4 rounded-[8px] border border-warn/30 bg-warn-soft px-3 py-2.5">
                  <p className="text-[12px] font-medium text-warn">Sleep is not being held</p>
                  <p className="mt-0.5 text-[12px] text-muted">
                    A project asked to keep this machine awake, but the system would not allow it.
                    The machine may still sleep.
                  </p>
                </div>
              )}
            </Card>

            <Card className="lg:col-span-2">
              <CardHeader title="Recent power changes" />
              {journal.length === 0 ? (
                <p className="px-4 py-3 text-[12px] text-muted">
                  Nothing yet. Only changes are recorded — a machine doing the same thing for an
                  hour produces one line, not eighteen hundred.
                </p>
              ) : (
                <ul className="py-1">
                  {journal.map((entry) => (
                    <li key={entry.seq} className="border-b border-edge/60 py-2 last:border-b-0">
                      <div className="flex items-baseline justify-between gap-3">
                        <span className="text-[13px] text-ink">{entry.summary}</span>
                        <span className="shrink-0 tabular text-[11px] text-muted">{entry.at}</span>
                      </div>
                      <p className="mt-0.5 text-[12px] text-muted">{entry.reason}</p>
                    </li>
                  ))}
                </ul>
              )}
            </Card>
          </div>
        )}

        {tab === 'storage' && (
          <div className="grid gap-4 lg:grid-cols-2">
            <Card>
              <CardHeader
                title="Locations"
                actions={
                  <Button size="sm" icon="external" onClick={openProjectFolder}>
                    Open a project folder
                  </Button>
                }
              />
              <div className="py-1">
                {pathRow('Data', settings?.dataDir)}
                {pathRow('Projects', settings?.projectsDir)}
                {pathRow('Logs', settings?.logsDir)}
                {pathRow('Backups', settings?.backupsDir)}
              </div>
              <p className="border-t border-edge px-4 py-2.5 text-[12px] text-muted">
                Nothing leaves this machine unless you check for updates.
              </p>
            </Card>

            <Card>
              <CardHeader title="Disk" subtitle={metrics?.diskMount} />
              {metrics === null ? (
                <div className="p-4">
                  <Skeleton className="h-16" />
                </div>
              ) : (
                <div className="py-1">
                  <DataRow label="Used" value={formatBytes(metrics.diskUsedBytes)} />
                  <DataRow label="Total" value={formatBytes(metrics.diskTotalBytes)} />
                  <DataRow
                    label="Free"
                    value={formatBytes(metrics.diskTotalBytes - metrics.diskUsedBytes)}
                  />
                </div>
              )}
              <p className="border-t border-edge px-4 py-2.5 text-[12px] text-muted">
                Backups are not built yet. The folder above is reserved for them.
              </p>
            </Card>
          </div>
        )}

        {tab === 'networking' && (
          <Card className="max-w-xl">
            <CardHeader
              title="Host port pool"
              subtitle="Ports projects are given on this machine"
            />
            <div className="py-1">
              <DataRow
                label="Range"
                value={settings ? `${settings.portPoolStart}–${settings.portPoolEnd}` : '—'}
              />
              <DataRow label="Ports available" value={settings ? settings.portPoolSize : '—'} />
            </div>
            <p className="border-t border-edge px-4 py-2.5 text-[12px] text-muted">
              A project is allocated a free port from this range when it first starts. The port it
              received is on the project&apos;s Networking tab.
            </p>
          </Card>
        )}

        {/* This tab describes the update *settings*. What an update is doing
            lives in the update manager, which is the one surface that renders
            it — three panels each drawing the same install from the same store
            is what the manager replaced. */}
        {tab === 'updates' && (
          <Card className="max-w-xl">
            <CardHeader
              title="Updates"
              actions={
                <Button
                  size="sm"
                  icon="refresh"
                  pending={checking}
                  disabled={installBusy(update)}
                  onClick={onOpenUpdates}
                >
                  {checking ? 'Checking…' : 'Check now'}
                </Button>
              }
            />
            <div className="py-1">
              <DataRow label="Installed version" value={status?.appVersion ?? '—'} />
              <DataRow label="Release channel" value="stable" />
              <DataRow label="Check on startup" value="on" />
              <DataRow label="Check while running" value="every 6 hours" />
              <DataRow label="Signature check" value="required" />
            </div>

            <p className="border-t border-edge px-4 py-2.5 text-[12px] text-muted">
              Every download is checked against the signing key built into this application before
              it is installed. The release channel is fixed to stable in this build.
            </p>
          </Card>
        )}

        {tab === 'logging' && (
          <div className="grid gap-4 lg:grid-cols-2">
            <Card className="max-w-xl">
              <CardHeader title="Logging" subtitle="What the core writes to disk" />
              <div className="py-1">
                <DataRow label="Level" value={settings?.logLevel ?? '—'} />
                <DataRow
                  label="Format"
                  value={settings ? (settings.logJson ? 'structured JSON' : 'plain text') : '—'}
                />
                <DataRow
                  label="Kept for"
                  value={settings ? `${settings.logRetentionDays} days` : '—'}
                />
                {pathRow('Directory', settings?.logsDir)}
              </div>
              <p className="border-t border-edge px-4 py-2.5 text-[12px] text-muted">
                Reading those files back into this window is not built yet, so nothing is shown here
                that has not been read.
              </p>
            </Card>
            <Card className="max-w-xl">
              <CardHeader
                title="This launch"
                subtitle="Which stage took time, so a slow start names its cause"
              />
              <div className="py-1">
                <DataRow label="Total" value={launch ? `${launch.totalMs} ms` : '—'} />
                {(launch?.stages ?? []).map((stage) => (
                  <DataRow
                    key={stage.name}
                    label={stage.name}
                    value={`${stage.durationMs} ms · ${stage.outcome}`}
                  />
                ))}
              </div>
              <p className="border-t border-edge px-4 py-2.5 text-[12px] text-muted">
                Stages after the window opened (Docker, the full machine scan) appear here once they
                finish. A stage over 500 ms is also written as a warning in the log.
              </p>
            </Card>
          </div>
        )}

        {tab === 'about' && (
          <div className="grid gap-4 lg:grid-cols-2">
            <Card>
              <CardHeader title="Application" />
              <div className="py-1">
                <DataRow label="Version" value={status?.appVersion ?? '—'} />
                <DataRow label="Database schema" value={status?.schemaVersion ?? '—'} />
                <DataRow
                  label="Uptime"
                  value={status ? formatDuration(status.uptimeSeconds) : '—'}
                />
                <DataRow label="Started" value={status?.startedAt ?? '—'} />
              </div>
            </Card>

            <Card>
              <CardHeader title="Machine" />
              {metrics === null ? (
                <div className="p-4">
                  <Skeleton className="h-24" />
                </div>
              ) : (
                <div className="py-1">
                  <DataRow label="Logical cores" value={metrics.cpuCount} />
                  <DataRow label="Memory" value={formatBytes(metrics.memoryTotalBytes)} />
                  <DataRow label="Projects volume" value={metrics.diskMount || '—'} mono />
                </div>
              )}
              <div className="border-t border-edge px-4 py-3">
                <Button
                  icon="copy"
                  onClick={() =>
                    copy(JSON.stringify({ status, settings, metrics }, null, 2), 'Diagnostics')
                  }
                >
                  Copy diagnostics
                </Button>
              </div>
            </Card>
          </div>
        )}
      </div>

      {confirmReset && (
        <ConfirmDialog
          title="Reset layout and preferences?"
          description="Panel sizes, the sidebar state and this window's preferences return to their defaults. Projects and files are untouched."
          confirmLabel="Reset"
          onCancel={() => setConfirmReset(false)}
          onConfirm={() => {
            setConfirmReset(false);
            onResetLayout();
            toast.success('Preferences reset');
          }}
        />
      )}
    </PageShell>
  );
}

// --------------------------------------------------------------- appearance

/**
 * The Appearance tab.
 *
 * Every control here writes a token and takes effect immediately — there is no
 * Apply button, because a theme you cannot see until you confirm it is a theme
 * you have to confirm twice. The swatches are the real values from the theme
 * table, so a card can never advertise a colour the theme does not use.
 */
function AppearancePanel({
  appearance,
  onChange,
}: {
  appearance: Appearance;
  onChange: (next: Partial<Appearance>) => void;
}) {
  return (
    <div className="animate-view flex flex-col gap-4">
      <Card>
        <CardHeader title="Theme" subtitle="Applies everywhere except the Discord panel" />
        <ThemeBrowser value={appearance.theme} onChange={(theme) => onChange({ theme })} />
      </Card>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader title="Accent" subtitle="The primary action, links and the active item" />
          <div className="flex flex-wrap gap-2 px-4 pb-4">
            {ACCENTS.map((accent) => {
              const active = appearance.accent === accent.id;
              return (
                <button
                  key={accent.id}
                  type="button"
                  aria-pressed={active}
                  aria-label={accent.label}
                  title={
                    accent.id === 'auto'
                      ? 'Use the accent this theme was designed with'
                      : accent.label
                  }
                  onClick={() => onChange({ accent: accent.id })}
                  className={`grid h-9 w-9 place-items-center rounded-full border-2 ${
                    active ? 'border-ink' : 'border-transparent hover:border-edge-strong'
                  }`}
                >
                  <span
                    aria-hidden
                    className="grid h-6 w-6 place-items-center rounded-full text-white"
                    style={
                      // Auto has no colour of its own: it shows the accent the
                      // current theme brought, which is exactly what choosing it
                      // means.
                      accent.value === null
                        ? {
                            background: 'var(--color-accent)',
                            boxShadow: 'inset 0 0 0 2px var(--color-canvas)',
                          }
                        : { background: accent.value }
                    }
                  >
                    {active && <Icon name="check" size={12} />}
                  </span>
                </button>
              );
            })}
          </div>
          <p className="px-4 pb-4 text-[12px] leading-snug text-muted">
            Theme default keeps the accent each theme was designed with. Choosing a colour applies
            it over every theme.
          </p>
        </Card>

        <Card>
          <CardHeader title="Density" subtitle="Row heights and page gutters, not text size" />
          <div className="flex gap-2 px-4 pb-4">
            {(
              [
                ['comfortable', 'Comfortable'],
                ['compact', 'Compact'],
              ] as [Density, string][]
            ).map(([id, label]) => (
              <button
                key={id}
                type="button"
                aria-pressed={appearance.density === id}
                onClick={() => onChange({ density: id })}
                className={`h-9 flex-1 rounded-[8px] border text-[13px] ${
                  appearance.density === id
                    ? 'border-accent bg-accent-soft text-ink'
                    : 'border-edge bg-raised text-muted hover:text-ink'
                }`}
              >
                {label}
              </button>
            ))}
          </div>
        </Card>

        <Card>
          <CardHeader title="Text size" subtitle={`${appearance.fontScale}% of the default`} />
          <div className="px-4 pb-4">
            <input
              type="range"
              min={90}
              max={120}
              step={5}
              value={appearance.fontScale}
              aria-label="Text size"
              onChange={(event) => onChange({ fontScale: Number(event.target.value) })}
              className="w-full accent-[var(--color-accent)]"
            />
            <div className="mt-1 flex justify-between text-[11px] text-faint">
              <span>90%</span>
              <span>120%</span>
            </div>
          </div>
        </Card>

        <Card>
          <CardHeader title="Motion" subtitle="How much the interface animates" />
          <div className="flex flex-col gap-2 px-4 pb-4">
            <div className="flex gap-2">
              {(
                [
                  ['full', 'Full'],
                  ['reduced', 'Reduced'],
                  ['off', 'Off'],
                ] as [MotionLevel, string][]
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  aria-pressed={appearance.motion === id}
                  onClick={() => onChange({ motion: id })}
                  className={`h-9 flex-1 rounded-[8px] border text-[13px] ${
                    appearance.motion === id
                      ? 'border-accent bg-accent-soft text-ink'
                      : 'border-edge bg-raised text-muted hover:text-ink'
                  }`}
                >
                  {label}
                </button>
              ))}
            </div>
            <p className="text-[12px] leading-snug text-muted">
              If your system asks for reduced motion, that wins over Full — the setting here can
              only ever remove animation, never force it on.
            </p>
          </div>
        </Card>
      </div>
    </div>
  );
}
