/**
 * Creating a project.
 *
 * A four-step wizard rather than one long modal: the old form put a name, a
 * source, four remote fields and a runtime picker in a single scroll, and the
 * only way to know which parts applied was to read all of them.
 *
 * The steps are the four questions the core actually needs answered. A hosting
 * panel would normally also ask for install and build commands, resource limits
 * and a restart policy — the core derives those from the runtime template and
 * has no command that accepts them, so asking would be collecting answers with
 * nowhere to put them. The review step says what will be derived instead.
 */
import { useEffect, useState } from 'react';

import {
  errorMessage,
  githubCliStatus,
  supportedRuntimes,
  type CreatedProject,
  type GitHubCliStatus,
  type RuntimeOption,
  type SourceKind,
} from '../api';
import {
  canCreate,
  emptyDraft,
  isGitLike,
  isRemote,
  progressLabel,
  STEPS,
  stepForError,
  stepIsValid,
  takesToken,
  validate,
  type Draft,
  type StepId,
} from '../lib/wizard';
import { runtimeLabel } from '../lib/projectList';
import Icon, { type IconName } from '../ui/Icon';
import { Modal } from '../ui/overlays';
import Select from '../ui/Select';
import { Badge, Button, DataRow, TextInput, Toggle } from '../ui/primitives';

const SOURCES: {
  id: SourceKind;
  label: string;
  hint: string;
  icon: IconName;
}[] = [
  {
    id: 'EMPTY',
    label: 'Empty project',
    hint: 'Start with nothing and add files yourself',
    icon: 'folder',
  },
  {
    id: 'GIT_CLONE',
    label: 'Git repository',
    hint: 'Clone from GitHub or any https remote',
    icon: 'git',
  },
  {
    id: 'GITHUB_CLI',
    label: 'GitHub CLI',
    hint: 'owner/repo, using your gh login',
    icon: 'git',
  },
  {
    id: 'REMOTE_ARCHIVE',
    label: 'Archive URL',
    hint: 'Download a .zip or .tar.gz',
    icon: 'download',
  },
];

export default function NewProjectWizard({
  onClose,
  onCreate,
  onCreated,
}: {
  onClose: () => void;
  onCreate: (draft: Draft) => Promise<CreatedProject>;
  onCreated: (project: CreatedProject, startNow: boolean) => void;
}) {
  const [step, setStep] = useState<StepId>('source');
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [runtimes, setRuntimes] = useState<RuntimeOption[]>([]);
  const [gh, setGh] = useState<GitHubCliStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  /** Fields the user has finished with, so errors appear after typing not during. */
  const [touched, setTouched] = useState<Record<string, boolean>>({});

  const errors = validate(draft);
  const index = STEPS.findIndex((entry) => entry.id === step);
  const isLast = step === 'review';

  // The override list comes from the core, so the wizard cannot offer a
  // language the planner would refuse.
  useEffect(() => {
    supportedRuntimes()
      .then(setRuntimes)
      .catch(() => setRuntimes([]));
  }, []);

  // Asked once: spawning `gh` is not free and the answer does not change while
  // the dialog is open.
  useEffect(() => {
    githubCliStatus()
      .then(setGh)
      .catch(() => setGh({ installed: false, account: null, hint: null }));
  }, []);

  const patch = (next: Partial<Draft>) => setDraft((current) => ({ ...current, ...next }));

  async function create() {
    setBusy(true);
    setFailure(null);
    try {
      const created = await onCreate(draft);
      onCreated(created, draft.startNow);
      onClose();
    } catch (error) {
      // Shown in the dialog rather than closing it, so nothing typed is lost —
      // which matters most for the fields nobody can retype from memory.
      setFailure(errorMessage(error));
      setBusy(false);
    }
  }

  return (
    <Modal
      title="New project"
      description={STEPS[index]?.description}
      size="lg"
      onClose={busy ? () => undefined : onClose}
      footer={
        <>
          {/* Cancel stays available while creating: the core writes the project
              row only once the files are in place, so backing out of a clone
              leaves nothing half-made. */}
          <Button onClick={onClose} disabled={busy && isLast === false}>
            Cancel
          </Button>
          <span className="flex-1" />
          {index > 0 && (
            <Button disabled={busy} onClick={() => setStep(STEPS[index - 1]?.id ?? 'source')}>
              Back
            </Button>
          )}
          {isLast ? (
            <Button
              variant="primary"
              disabled={!canCreate(draft) || busy}
              onClick={() => void create()}
            >
              {busy ? progressLabel(draft.source) : 'Create project'}
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={!stepIsValid(draft, step)}
              onClick={() => {
                setTouched((current) => ({ ...current, [step]: true }));
                setStep(STEPS[index + 1]?.id ?? 'review');
              }}
            >
              Next
            </Button>
          )}
        </>
      }
    >
      <ol className="mb-5 flex items-center gap-1">
        {STEPS.map((entry, at) => {
          const done = at < index;
          const current = at === index;
          return (
            <li key={entry.id} className="flex min-w-0 flex-1 items-center gap-2">
              <button
                type="button"
                // Only backwards: skipping ahead past a required field is how
                // a wizard ends up on Review with nothing filled in.
                disabled={at > index || busy}
                onClick={() => setStep(entry.id)}
                className="flex min-w-0 items-center gap-2 disabled:cursor-default"
              >
                <span
                  className={`grid h-6 w-6 shrink-0 place-items-center rounded-full text-[11px] font-semibold ${
                    current
                      ? 'bg-accent text-canvas'
                      : done
                        ? 'bg-ok-soft text-ok'
                        : 'border border-edge text-faint'
                  }`}
                >
                  {done ? <Icon name="check" size={12} /> : at + 1}
                </span>
                <span className={`truncate text-[13px] ${current ? 'text-ink' : 'text-muted'}`}>
                  {entry.title}
                </span>
              </button>
              {at < STEPS.length - 1 && <span className="h-px flex-1 bg-edge" />}
            </li>
          );
        })}
      </ol>

      {step === 'source' && (
        <div className="space-y-4">
          <div className="grid gap-2 sm:grid-cols-2">
            {SOURCES.map((option) => {
              const selected = draft.source === option.id;
              return (
                <button
                  key={option.id}
                  type="button"
                  onClick={() => patch({ source: option.id })}
                  className={`flex items-start gap-2.5 rounded-[10px] border px-3 py-2.5 text-left ${
                    selected
                      ? 'border-accent bg-accent-soft'
                      : 'border-edge bg-canvas hover:border-edge-strong'
                  }`}
                >
                  <span className={`mt-0.5 ${selected ? 'text-accent' : 'text-muted'}`}>
                    <Icon name={option.icon} size={16} />
                  </span>
                  <span className="min-w-0">
                    <span className="block text-[13px] font-medium text-ink">{option.label}</span>
                    <span className="block text-[12px] text-muted">{option.hint}</span>
                  </span>
                </button>
              );
            })}
          </div>

          <p className="text-[12px] text-faint">
            Importing a folder or a local archive is not offered here: the core creates a project
            from an empty folder or a remote address, and files from this machine are dragged into
            the editor afterwards.
          </p>

          {isRemote(draft.source) && (
            <div className="space-y-4 rounded-[10px] border border-edge bg-canvas p-3">
              {draft.source === 'GITHUB_CLI' && gh && (
                <p
                  className={`rounded-[8px] px-3 py-2 text-[12px] leading-relaxed ${
                    gh.installed && gh.account ? 'bg-ok-soft text-ok' : 'bg-warn-soft text-warn'
                  }`}
                >
                  {gh.installed && gh.account
                    ? `Using your gh login as ${gh.account}. Private repositories you can see will clone without a token.`
                    : (gh.hint ??
                      'The GitHub CLI could not be used. Use “Git repository” with a token instead.')}
                </p>
              )}

              <TextInput
                label={
                  draft.source === 'GIT_CLONE'
                    ? 'Repository address'
                    : draft.source === 'GITHUB_CLI'
                      ? 'Repository'
                      : 'Archive address'
                }
                value={draft.url}
                mono
                onChange={(url) => patch({ url })}
                error={touched.source ? errors.url : undefined}
                placeholder={
                  draft.source === 'GIT_CLONE'
                    ? 'https://github.com/owner/repo.git'
                    : draft.source === 'GITHUB_CLI'
                      ? 'owner/repo'
                      : 'https://example.com/release.zip'
                }
                hint={
                  draft.source === 'GITHUB_CLI'
                    ? 'An owner/repo name, or a github.com address. A link to a file or a pull request is refused.'
                    : 'Must be https. Addresses inside this machine or your own network are refused.'
                }
              />

              {isGitLike(draft.source) && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <TextInput
                    label="Branch or tag"
                    value={draft.gitRef}
                    mono
                    onChange={(gitRef) => patch({ gitRef })}
                    placeholder="Default branch"
                  />
                  <TextInput
                    label="Folder inside the repository"
                    value={draft.subdirectory}
                    mono
                    onChange={(subdirectory) => patch({ subdirectory })}
                    placeholder="Optional"
                  />
                </div>
              )}

              {takesToken(draft.source) && (
                <TextInput
                  label="Access token"
                  type="password"
                  value={draft.token}
                  mono
                  onChange={(token) => patch({ token })}
                  placeholder="Only for a private remote"
                  hint="Used for this download only. It is not saved — there is nowhere to keep it encrypted until the key store is built, so nothing is written rather than a token stored in the clear."
                />
              )}
            </div>
          )}
        </div>
      )}

      {step === 'details' && (
        <div className="space-y-4">
          <TextInput
            label="Name"
            autoFocus
            value={draft.name}
            maxLength={60}
            onChange={(name) => patch({ name })}
            error={touched.details ? errors.name : undefined}
            placeholder="My Discord bot"
            hint="Shown throughout the application. The folder name is derived from it."
          />
          <TextInput
            label="Description"
            value={draft.description}
            maxLength={200}
            onChange={(description) => patch({ description })}
            placeholder="Optional"
          />
        </div>
      )}

      {step === 'runtime' && (
        <div className="space-y-4">
          {isRemote(draft.source) ? (
            <div className="space-y-2">
              <RuntimeChoice
                selected={draft.runtime === null}
                title="Detect automatically"
                hint="Read the files once they arrive and decide"
                onSelect={() => patch({ runtime: null })}
              />
              <RuntimeChoice
                selected={draft.runtime !== null}
                title="Choose it myself"
                hint="Override what detection would pick"
                onSelect={() => patch({ runtime: runtimes[0]?.id ?? 'NODEJS' })}
              />
            </div>
          ) : (
            <p className="text-[13px] text-muted">
              An empty project has no files to read, so choose the runtime yourself.
            </p>
          )}

          {(draft.runtime !== null || !isRemote(draft.source)) && (
            <Select
              label="Runtime"
              value={draft.runtime}
              onChange={(runtime) => patch({ runtime })}
              error={touched.runtime ? errors.runtime : undefined}
              placeholder="Choose a runtime…"
              options={runtimes.map((option) => ({
                value: option.id,
                label: option.label,
              }))}
              hint="The list comes from the core, so every option here can actually be built."
            />
          )}
        </div>
      )}

      {step === 'review' && (
        <div className="space-y-4">
          <div className="rounded-[10px] border border-edge bg-canvas px-3 py-1">
            <DataRow label="Name" value={draft.name.trim() || '—'} />
            {draft.description.trim() && (
              <DataRow label="Description" value={draft.description.trim()} />
            )}
            <DataRow
              label="Source"
              value={SOURCES.find((option) => option.id === draft.source)?.label ?? draft.source}
            />
            {isRemote(draft.source) && <DataRow label="Address" value={draft.url.trim()} mono />}
            {isGitLike(draft.source) && draft.gitRef.trim() && (
              <DataRow label="Branch or tag" value={draft.gitRef.trim()} mono />
            )}
            {isGitLike(draft.source) && draft.subdirectory.trim() && (
              <DataRow label="Subdirectory" value={draft.subdirectory.trim()} mono />
            )}
            {takesToken(draft.source) && (
              <DataRow
                label="Token"
                value={draft.token.trim().length > 0 ? 'provided, not saved' : 'none'}
              />
            )}
            <DataRow
              label="Runtime"
              value={
                draft.runtime === null ? 'detected from the files' : runtimeLabel(draft.runtime)
              }
            />
          </div>

          <div className="rounded-[10px] border border-edge bg-canvas px-3 py-1">
            <Toggle
              checked={draft.startNow}
              onChange={(startNow) => patch({ startNow })}
              label="Start the project once it is created"
              description="A first start builds the image, which can take a few minutes."
            />
          </div>

          <p className="text-[12px] leading-relaxed text-faint">
            The install, build and start commands, the port, the resource limits and the restart
            policy come from the runtime template the core picks. They are shown on the
            project&apos;s page once it exists.
          </p>

          {Object.entries(errors).length > 0 && (
            <div className="rounded-[10px] border border-warn/30 bg-warn-soft px-3 py-2.5">
              <p className="mb-1 text-[12px] font-medium text-warn">Still needed</p>
              <ul className="space-y-1">
                {Object.entries(errors).map(([field, message]) => (
                  <li key={field} className="flex items-center gap-2 text-[12px] text-muted">
                    <span className="flex-1">{message}</span>
                    <button
                      type="button"
                      onClick={() => setStep(stepForError(field as keyof typeof errors))}
                      className="text-accent hover:underline"
                    >
                      Fix
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {busy && (
            <div className="flex items-center gap-2.5 rounded-[10px] border border-edge bg-canvas px-3 py-2.5">
              <span className="h-2 w-2 animate-pulse rounded-full bg-accent" aria-hidden />
              <p className="text-[13px] text-muted">{progressLabel(draft.source)}</p>
            </div>
          )}

          {failure && (
            <div className="rounded-[10px] border border-danger/30 bg-danger-soft px-3 py-2.5">
              <p className="text-[12px] font-medium text-danger">The project was not created</p>
              <p className="mt-0.5 text-[12px] break-words text-muted">{failure}</p>
            </div>
          )}
        </div>
      )}
    </Modal>
  );
}

function RuntimeChoice({
  selected,
  title,
  hint,
  onSelect,
}: {
  selected: boolean;
  title: string;
  hint: string;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={`flex w-full items-center gap-2.5 rounded-[10px] border px-3 py-2.5 text-left ${
        selected ? 'border-accent bg-accent-soft' : 'border-edge bg-canvas hover:border-edge-strong'
      }`}
    >
      <span
        className={`grid h-4 w-4 shrink-0 place-items-center rounded-full border ${
          selected ? 'border-accent' : 'border-edge-strong'
        }`}
      >
        {selected && <span className="h-2 w-2 rounded-full bg-accent" />}
      </span>
      <span className="min-w-0">
        <span className="block text-[13px] font-medium text-ink">{title}</span>
        <span className="block text-[12px] text-muted">{hint}</span>
      </span>
    </button>
  );
}

/** Shown after the dialog closes: what the files turned out to be. */
export function CreatedSummary({
  created,
  onDismiss,
  onOpen,
}: {
  created: CreatedProject;
  onDismiss: () => void;
  onOpen: () => void;
}) {
  return (
    <div className="mb-4 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-[12px] border border-ok/30 bg-ok-soft px-4 py-3">
      <span className="text-ok">
        <Icon name="check-circle" size={16} />
      </span>
      <div className="min-w-[220px] flex-1">
        <p className="text-[13px] font-medium text-ink">{created.displayName} created</p>
        <p className="mt-0.5 text-[12px] text-muted">
          {created.detected
            ? `Detected ${created.languages.join(' + ')} — built as ${runtimeLabel(created.runtime)}.`
            : `Built as ${runtimeLabel(created.runtime)}.`}
        </p>
        {created.notes.length > 0 && (
          <ul className="mt-1 space-y-0.5">
            {created.notes.map((note) => (
              <li key={note} className="text-[12px] text-muted">
                · {note}
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Badge tone="ok">{runtimeLabel(created.runtime)}</Badge>
        <Button size="sm" onClick={onOpen}>
          Open
        </Button>
        <Button size="sm" variant="ghost" onClick={onDismiss}>
          Dismiss
        </Button>
      </div>
    </div>
  );
}
