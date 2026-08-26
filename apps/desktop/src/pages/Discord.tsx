/**
 * The Discord integration.
 *
 * This screen used to be a setup flow with its first step blocked, because the
 * gateway connection was not written. It is now the thing itself: bots this
 * installation holds a token for, each one started and stopped independently,
 * with what its connection is actually doing.
 *
 * Three things shape the layout:
 *
 * * **A bot is a running thing, like a project.** It gets the same vocabulary —
 *   a status dot, uptime, Start and Stop — because a user who can read the
 *   project list should not have to learn a second one here.
 * * **Nothing is shown that is not true.** There is no mock-up of a linked
 *   server any more. An installation with no bots gets an empty state that says
 *   how to add one, not a picture of somebody else's.
 * * **A token is the most sensitive thing this window accepts.** The field is a
 *   password field, it is cleared the moment it is submitted, and the screen
 *   refuses to offer it at all when secure storage could not be opened.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  addDiscordBot,
  discordBots,
  discordReadiness,
  forgetDiscordBot,
  listProjects,
  setDiscordBotProjects,
  startDiscordBot,
  stopDiscordBot,
  updateDiscordBot,
  type DiscordBot,
  type DiscordReadiness,
  type ProjectSummary,
} from '../api';
import Icon from '../ui/Icon';
import {
  Badge,
  Banner,
  Button,
  Card,
  CardHeader,
  EmptyState,
  IconButton,
  TextInput,
  Toggle,
  PageShell,
} from '../ui/primitives';

/** How often the list re-reads connection status while the screen is open. */
const POLL_MS = 2000;

const PERMISSIONS = [
  'Manage Channels — to create the log and control channels',
  'Send Messages — to post status and control panels',
  'Embed Links — for the control panel layout',
  'Read Message History — to update a panel it posted earlier',
];

type Tone = 'ok' | 'warn' | 'danger' | 'neutral';

function toneFor(status: string): Tone {
  if (status === 'connected') return 'ok';
  if (status === 'connecting') return 'warn';
  if (status === 'failed') return 'danger';
  return 'neutral';
}

function labelFor(status: string): string {
  if (status === 'connected') return 'Connected';
  if (status === 'connecting') return 'Connecting';
  if (status === 'failed') return 'Failed';
  return 'Stopped';
}

/** `4h 12m`, matching how the project list writes an uptime. */
function formatUptime(seconds: number | null): string {
  if (seconds === null) return '—';
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m`;
  return `${seconds}s`;
}

export default function Discord() {
  const [bots, setBots] = useState<DiscordBot[] | null>(null);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [readiness, setReadiness] = useState<DiscordReadiness | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  // Which bot has a request in flight, so only its own button spins rather
  // than the whole list going inert.
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async (): Promise<void> => {
    try {
      const [list, ready, allProjects] = await Promise.all([
        discordBots(),
        discordReadiness(),
        listProjects(),
      ]);
      setBots(list);
      setReadiness(ready);
      setProjects(allProjects);
    } catch (cause) {
      setError(String(cause));
      setBots([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Connection status changes without the user doing anything — a network drops,
  // a token is revoked — so the screen keeps asking while it is open.
  useEffect(() => {
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const act = useCallback(
    async (id: string, action: () => Promise<unknown>): Promise<void> => {
      setBusy(id);
      setError(null);
      try {
        await action();
      } catch (cause) {
        setError(String(cause));
      } finally {
        setBusy(null);
        await refresh();
      }
    },
    [refresh],
  );

  const connected = bots?.filter((bot) => bot.status === 'connected').length ?? 0;
  const canStore = readiness?.canStoreSecrets ?? true;

  return (
    <PageShell title="Discord" description="Watch and control projects from a Discord server.">
      {/* Everything below opts out of the user's theme. Discord's surface is
          Discord's identity, so this panel stays Discord-coloured on Light,
          Amber and Nord alike — see `.discord-scope` in `styles.css`. */}
      <div className="discord-scope animate-view rounded-card">
        {error && (
          <div className="mb-4">
            <Banner
              tone="danger"
              title="That did not work"
              description={error}
              onDismiss={() => setError(null)}
            />
          </div>
        )}

        {!canStore && (
          <div className="mb-4">
            <Banner
              tone="warn"
              title="This machine cannot store a bot token"
              description="Secure storage could not be opened, so there is nowhere to keep a token safely. Adding a bot is disabled rather than saving one in the clear. The application log says why."
            />
          </div>
        )}

        <Card className="mb-4">
          <div className="flex flex-wrap items-center gap-x-4 gap-y-3 px-4 py-3.5">
            <span className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] border border-edge bg-raised text-muted">
              <Icon name="discord" size={18} />
            </span>
            <div className="min-w-[220px] flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <p className="text-[14px] font-medium text-ink">
                  {bots === null
                    ? 'Loading'
                    : bots.length === 0
                      ? 'No bots yet'
                      : `${bots.length} ${bots.length === 1 ? 'bot' : 'bots'}`}
                </p>
                {connected > 0 && (
                  <Badge tone="ok" dot>
                    {connected} connected
                  </Badge>
                )}
              </div>
              <p className="mt-0.5 text-[13px] text-muted">
                Each bot is its own connection, started and stopped on its own. They run on this
                machine and stay up until you stop them.
              </p>
            </div>
            <Button
              icon="plus"
              variant="primary"
              disabled={!canStore || adding}
              title={canStore ? undefined : 'Secure storage is unavailable on this machine'}
              onClick={() => setAdding(true)}
            >
              Add a bot
            </Button>
          </div>
        </Card>

        {adding && (
          <AddBotForm
            onCancel={() => setAdding(false)}
            onAdded={async () => {
              setAdding(false);
              await refresh();
            }}
          />
        )}

        <div className="grid gap-4 lg:grid-cols-[1fr_320px]">
          <Card>
            <CardHeader
              title="Bots"
              subtitle={
                readiness?.keyBackend
                  ? `Tokens encrypted, key held by ${readiness.keyBackend === 'os-keychain' ? 'the system keychain' : 'an owner-only file'}`
                  : 'Tokens are encrypted before they are stored'
              }
            />

            {bots === null ? (
              <div className="px-4 py-8 text-center text-[13px] text-muted">Loading…</div>
            ) : bots.length === 0 ? (
              <div className="px-4 py-6">
                <EmptyState
                  icon="discord"
                  title="No bots connected"
                  description="Create an application in Discord's developer portal, copy its bot token, and add it here. The token is checked with Discord before it is saved."
                />
              </div>
            ) : (
              <ul>
                {bots.map((bot) => (
                  <BotRow
                    key={bot.id}
                    bot={bot}
                    projects={projects}
                    busy={busy === bot.id}
                    onStart={() => act(bot.id, () => startDiscordBot(bot.id))}
                    onStop={() => act(bot.id, () => stopDiscordBot(bot.id))}
                    onForget={() => act(bot.id, () => forgetDiscordBot(bot.id))}
                    onAutostart={(next) =>
                      act(bot.id, () => updateDiscordBot(bot.id, bot.label, next))
                    }
                    onProjects={(ids) => act(bot.id, () => setDiscordBotProjects(bot.id, ids))}
                  />
                ))}
              </ul>
            )}
          </Card>

          <Card>
            <CardHeader title="Permissions it will ask for" />
            <ul className="px-4 py-2">
              {PERMISSIONS.map((permission) => (
                <li
                  key={permission}
                  className="flex items-start gap-2 border-b border-edge/60 py-2 text-[12px] text-muted last:border-b-0"
                >
                  <span className="mt-0.5 text-faint">
                    <Icon name="shield" size={13} />
                  </span>
                  {permission}
                </li>
              ))}
            </ul>
            <div className="border-t border-edge px-4 py-3">
              <p className="text-[12px] text-muted">
                The connection asks for no privileged intents, so nothing needs enabling in the
                developer portal beyond inviting the bot.
              </p>
            </div>
          </Card>
        </div>
      </div>
    </PageShell>
  );
}

// ------------------------------------------------------------------- one bot

function BotRow({
  bot,
  projects,
  busy,
  onStart,
  onStop,
  onForget,
  onAutostart,
  onProjects,
}: {
  bot: DiscordBot;
  projects: ProjectSummary[];
  busy: boolean;
  onStart: () => void;
  onStop: () => void;
  onForget: () => void;
  onAutostart: (next: boolean) => void;
  onProjects: (ids: string[]) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [picking, setPicking] = useState(false);
  const running = bot.status === 'connected' || bot.status === 'connecting';
  const tone = toneFor(bot.status);

  return (
    <li className="border-b border-edge/60 px-4 py-3.5 last:border-b-0">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <div className="min-w-[180px] flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-[13px] font-medium text-ink">{bot.label}</span>
            <Badge tone={tone} dot>
              {labelFor(bot.status)}
            </Badge>
            {bot.linkedServers > 0 && (
              <Badge tone="neutral">
                {bot.linkedServers} {bot.linkedServers === 1 ? 'server' : 'servers'}
              </Badge>
            )}
            <Badge tone={bot.projectIds.length > 0 ? 'neutral' : 'warn'}>
              {bot.projectIds.length === 0
                ? 'no projects'
                : `${bot.projectIds.length} ${bot.projectIds.length === 1 ? 'project' : 'projects'}`}
            </Badge>
          </div>

          <p className="mt-0.5 font-mono text-[11px] text-faint">{bot.applicationId}</p>

          {/* The failure is the most important thing on the row when there is
              one, so it reads as a sentence rather than a status word. */}
          {bot.failureReason && (
            <p className="mt-1 flex items-start gap-1.5 text-[12px] text-danger">
              <Icon name="alert" size={13} className="mt-px shrink-0" />
              {bot.failureReason}
            </p>
          )}
        </div>

        {bot.status === 'connected' && (
          <div className="text-right text-[12px]">
            <p className="tabular text-ink">{formatUptime(bot.uptimeSeconds)}</p>
            <p className="text-faint">
              {bot.reconnects === 0
                ? 'no reconnects'
                : `${bot.reconnects} reconnect${bot.reconnects === 1 ? '' : 's'}`}
            </p>
          </div>
        )}

        <div className="flex shrink-0 items-center gap-2">
          {running ? (
            <Button size="sm" icon="stop" pending={busy} onClick={onStop}>
              Stop
            </Button>
          ) : (
            <Button size="sm" icon="play" variant="primary" pending={busy} onClick={onStart}>
              Start
            </Button>
          )}
          <IconButton
            icon="trash"
            label={`Forget ${bot.label}`}
            size="sm"
            onClick={() => setConfirming(true)}
          />
        </div>
      </div>

      <div className="mt-2.5 flex flex-wrap items-center justify-between gap-x-6 gap-y-2">
        <button
          type="button"
          onClick={() => setPicking((open) => !open)}
          aria-expanded={picking}
          className="inline-flex items-center gap-1.5 text-[12px] text-muted hover:text-ink"
        >
          <Icon name={picking ? 'chevron-down' : 'chevron-right'} size={13} />
          {bot.projectIds.length === 0
            ? 'Choose which projects this bot reports on'
            : `Reporting on ${summarise(bot.projectIds, projects)}`}
        </button>
      </div>

      {picking && (
        <ProjectPicker
          projects={projects}
          selected={bot.projectIds}
          busy={busy}
          onApply={(ids) => {
            setPicking(false);
            onProjects(ids);
          }}
          onCancel={() => setPicking(false)}
        />
      )}

      <div className="mt-1">
        <Toggle checked={bot.autostart} onChange={onAutostart} label="Start with the application" />
      </div>

      {confirming && (
        <div className="mt-3">
          <Banner
            tone="danger"
            title={`Forget ${bot.label}?`}
            description="Its token is deleted and any servers linked through it are unlinked. The bot itself is untouched on Discord — you can add it again with the same token."
            actions={
              <>
                <Button size="sm" onClick={() => setConfirming(false)}>
                  Cancel
                </Button>
                <Button
                  size="sm"
                  variant="danger"
                  onClick={() => {
                    setConfirming(false);
                    onForget();
                  }}
                >
                  Forget it
                </Button>
              </>
            }
          />
        </div>
      )}
    </li>
  );
}

/** `api and 2 others`, so a long list does not push the row apart. */
function summarise(ids: string[], projects: ProjectSummary[]): string {
  const names = ids.map(
    (id) => projects.find((project) => project.id === id)?.displayName ?? 'a deleted project',
  );
  const [first, second] = names;
  if (first === undefined) return 'nothing';
  if (second === undefined) return first;
  if (names.length === 2) return `${first} and ${second}`;
  return `${first} and ${names.length - 1} others`;
}

/**
 * Which projects a bot covers.
 *
 * Applied as one set on a button press rather than saving per tick. Ticking
 * five boxes should be one decision and one write, not five — and a mid-list
 * failure that left three applied would be worse than none.
 */
function ProjectPicker({
  projects,
  selected,
  busy,
  onApply,
  onCancel,
}: {
  projects: ProjectSummary[];
  selected: string[];
  busy: boolean;
  onApply: (ids: string[]) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState<string[]>(selected);

  // Reopening after a change elsewhere should show what is stored now, not
  // what was drafted the last time this was open.
  useEffect(() => {
    setDraft(selected);
  }, [selected]);

  const toggle = (id: string): void => {
    setDraft((current) =>
      current.includes(id) ? current.filter((each) => each !== id) : [...current, id],
    );
  };

  const changed = draft.length !== selected.length || draft.some((id) => !selected.includes(id));

  if (projects.length === 0) {
    return (
      <p className="mt-2 rounded-[8px] border border-edge bg-canvas px-3 py-2.5 text-[12px] text-muted">
        There are no projects yet. Create one and it will appear here.
      </p>
    );
  }

  return (
    <div className="mt-2 rounded-[8px] border border-edge bg-canvas">
      <ul className="max-h-[200px] overflow-y-auto p-1.5">
        {projects.map((project) => {
          const ticked = draft.includes(project.id);
          return (
            <li key={project.id}>
              <button
                type="button"
                onClick={() => toggle(project.id)}
                aria-pressed={ticked}
                className="flex w-full items-center gap-2.5 rounded-[6px] px-2 py-1.5 text-left hover:bg-raised"
              >
                <span
                  aria-hidden
                  className={`grid h-4 w-4 shrink-0 place-items-center rounded-[4px] border ${
                    ticked ? 'border-accent bg-accent text-canvas' : 'border-edge-strong'
                  }`}
                >
                  {ticked && <Icon name="check" size={11} />}
                </span>
                <span className="min-w-0 flex-1 truncate text-[13px] text-ink">
                  {project.displayName}
                </span>
                <span className="shrink-0 font-mono text-[11px] text-faint">{project.slug}</span>
              </button>
            </li>
          );
        })}
      </ul>
      <div className="flex justify-end gap-2 border-t border-edge px-3 py-2">
        <Button size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          size="sm"
          variant="primary"
          pending={busy}
          disabled={!changed}
          onClick={() => onApply(draft)}
        >
          Save
        </Button>
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ adding

/**
 * The token field.
 *
 * A password field, submitted on Enter, and cleared as soon as it is sent —
 * a bot token left sitting in a React state after the request is a token in a
 * heap snapshot for no reason.
 */
function AddBotForm({ onCancel, onAdded }: { onCancel: () => void; onAdded: () => Promise<void> }) {
  const [label, setLabel] = useState('');
  const [token, setToken] = useState('');
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const submit = async (): Promise<void> => {
    if (!token.trim() || pending) return;
    setPending(true);
    setFailure(null);
    try {
      await addDiscordBot(label, token);
      setToken('');
      await onAdded();
    } catch (cause) {
      if (mounted.current) {
        setFailure(String(cause));
        setPending(false);
      }
    }
  };

  return (
    <Card className="mb-4">
      <CardHeader
        title="Add a bot"
        subtitle="The token is checked with Discord before anything is saved"
      />
      <div className="grid gap-3 px-4 py-3.5 sm:grid-cols-2">
        <TextInput
          label="Name"
          value={label}
          onChange={setLabel}
          placeholder="Leave blank to use the bot's own name"
          maxLength={80}
          onKeyDown={(event) => {
            if (event.key === 'Enter') void submit();
          }}
        />
        <TextInput
          label="Bot token"
          type="password"
          mono
          autoFocus
          value={token}
          onChange={setToken}
          placeholder="Paste the token from the developer portal"
          error={failure ?? undefined}
          hint="Stored encrypted. It is never written to a log or shown again."
          onKeyDown={(event) => {
            if (event.key === 'Enter') void submit();
          }}
        />
      </div>
      <div className="flex flex-wrap justify-end gap-2 border-t border-edge px-4 py-3">
        <Button onClick={onCancel} disabled={pending}>
          Cancel
        </Button>
        <Button
          variant="primary"
          icon="check"
          pending={pending}
          disabled={!token.trim()}
          onClick={() => void submit()}
        >
          Check and save
        </Button>
      </div>
    </Card>
  );
}
