/**
 * The things the workspace has to ask before it acts.
 *
 * All modal on purpose. Deleting a folder, overwriting a file and reshaping an
 * import are the operations here that destroy or relocate something the user
 * cannot get back, and a toast that can be missed is the wrong shape for any
 * of them.
 */
import { useState, type ReactNode } from 'react';

import type { ConflictChoice } from './conflicts';
import { explainDetection, type ImportPlan } from './importPlan';
import Icon from './Icon';

function Modal({
  title,
  children,
  onDismiss,
}: {
  title: string;
  children: ReactNode;
  onDismiss: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/40 p-6"
      onMouseDown={onDismiss}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === 'Escape') onDismiss();
        }}
        className="w-[min(460px,92vw)] border border-vs-border bg-[#12182a] p-4 shadow-[0_16px_48px_rgba(0,0,0,0.6)]"
      >
        {children}
      </div>
    </div>
  );
}

export function ConfirmDialog({
  title,
  detail,
  confirmLabel,
  danger,
  onConfirm,
  onCancel,
}: {
  title: string;
  detail?: string;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <Modal title={title} onDismiss={onCancel}>
      <p className="text-[13px] text-vs-text">{title}</p>
      {detail && <p className="mt-1.5 text-[12px] text-vs-dim">{detail}</p>}
      <div className="mt-4 flex justify-end gap-2">
        <DialogButton onClick={onCancel}>Cancel</DialogButton>
        <DialogButton autoFocus primary danger={danger} onClick={onConfirm}>
          {confirmLabel}
        </DialogButton>
      </div>
    </Modal>
  );
}

/**
 * Asked once per drop, not once per file.
 *
 * The alternative — a prompt for every clashing name in a folder of two hundred
 * — is why people stop reading dialogs.
 */
export function ConflictDialog({
  conflicts,
  targetDirectory,
  onChoose,
  onCancel,
}: {
  conflicts: string[];
  targetDirectory: string;
  onChoose: (choice: ConflictChoice) => void;
  onCancel: () => void;
}) {
  const [first] = conflicts;

  return (
    <Modal title="Some files already exist" onDismiss={onCancel}>
      <p className="text-[13px] text-vs-text">
        {conflicts.length === 1
          ? `“${first}” already exists in ${targetDirectory || 'the project root'}.`
          : `${conflicts.length} of the files being imported already exist in ${
              targetDirectory || 'the project root'
            }.`}
      </p>

      {conflicts.length > 1 && (
        <ul className="mt-2 max-h-32 overflow-y-auto border border-vs-border bg-vs-editor p-2 font-mono text-[11px] text-vs-dim select-text">
          {conflicts.map((path) => (
            <li key={path} className="truncate">
              {path}
            </li>
          ))}
        </ul>
      )}

      <div className="mt-4 flex flex-wrap justify-end gap-2">
        <DialogButton onClick={onCancel}>Cancel</DialogButton>
        <DialogButton onClick={() => onChoose('skip')}>Skip existing</DialogButton>
        <DialogButton autoFocus primary onClick={() => onChoose('rename')}>
          Keep both
        </DialogButton>
        <DialogButton danger onClick={() => onChoose('replace')}>
          Replace
        </DialogButton>
      </div>
      <p className="mt-2 text-right text-[11px] text-vs-dim">
        Replacing deletes the existing file first. It cannot be undone.
      </p>
    </Modal>
  );
}

/**
 * What is about to be imported, and where it will land.
 *
 * Shown only when the decision is not obvious — a folder that will be unwrapped,
 * or several things at once. Dropping one ordinary folder is not worth a
 * dialog, and asking every time is how a confirmation stops being read.
 *
 * The detection is explained rather than asserted: each project says which
 * markers were found, and the choice of layout can be overridden.
 */
export function ImportPreviewDialog({
  plan,
  targetDirectory,
  onConfirm,
  onCancel,
}: {
  plan: ImportPlan;
  targetDirectory: string;
  /** `unwrap` is what the user settled on, which may differ from the plan. */
  onConfirm: (unwrap: boolean) => void;
  onCancel: () => void;
}) {
  const [unwrap, setUnwrap] = useState(plan.unwraps);
  const where = targetDirectory || 'the project root';
  const project = plan.projects[0];

  return (
    <Modal title="Import into this project" onDismiss={onCancel}>
      {plan.projects.length === 1 && project && (
        <>
          <p className="text-[13px] text-vs-text">
            <span className="font-medium">{project.name}</span> looks like a project.
          </p>
          <p className="mt-1 text-[12px] text-vs-dim">{explainDetection(project)}</p>

          <div className="mt-3 space-y-1.5">
            <LayoutChoice
              selected={unwrap}
              onSelect={() => setUnwrap(true)}
              title={`Import its contents into ${where}`}
              hint="The folder itself is not created. This is what opening a project means."
              preview={project.children.slice(0, 5)}
              prefix=""
            />
            <LayoutChoice
              selected={!unwrap}
              onSelect={() => setUnwrap(false)}
              title={`Keep the ${project.name} folder`}
              hint="Everything lands inside a folder of that name."
              preview={project.children.slice(0, 5)}
              prefix={`${project.name}/`}
            />
          </div>
        </>
      )}

      {plan.projects.length > 1 && (
        <>
          <p className="text-[13px] text-vs-text">
            {plan.projects.length} projects were detected. Each keeps its own folder — merging them
            would mix their files, and nothing says which one should own the root.
          </p>
          <ul className="mt-2 space-y-1">
            {plan.projects.map((candidate) => (
              <li key={candidate.path} className="text-[12px] text-vs-dim">
                <span className="font-mono text-vs-text">{candidate.name}/</span>{' '}
                {explainDetection(candidate)}
              </li>
            ))}
          </ul>
        </>
      )}

      {(plan.folders.length > 0 || plan.files.length > 0) && (
        <div className="mt-3 border-t border-vs-border pt-3">
          <p className="mb-1 text-[11px] tracking-wide text-vs-dim uppercase">Also importing</p>
          <ul className="max-h-28 space-y-0.5 overflow-y-auto">
            {[...plan.folders, ...plan.files].map((candidate) => (
              <li key={candidate.path} className="flex items-center gap-1.5 text-[12px]">
                <span className="text-vs-dim">
                  <Icon name={candidate.isDirectory ? 'folder' : 'file'} size={13} />
                </span>
                <span className="truncate font-mono text-vs-text">
                  {candidate.name}
                  {candidate.isDirectory ? '/' : ''}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="mt-4 flex justify-end gap-2">
        <DialogButton onClick={onCancel}>Cancel</DialogButton>
        <DialogButton autoFocus primary onClick={() => onConfirm(unwrap)}>
          Import
        </DialogButton>
      </div>
    </Modal>
  );
}

function LayoutChoice({
  selected,
  onSelect,
  title,
  hint,
  preview,
  prefix,
}: {
  selected: boolean;
  onSelect: () => void;
  title: string;
  hint: string;
  preview: string[];
  prefix: string;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={`flex w-full gap-2.5 border px-2.5 py-2 text-left ${
        selected ? 'border-accent bg-accent/10' : 'border-vs-border hover:border-white/25'
      }`}
    >
      <span
        className={`mt-0.5 grid h-3.5 w-3.5 shrink-0 place-items-center rounded-full border ${
          selected ? 'border-accent' : 'border-vs-dim'
        }`}
      >
        {selected && <span className="h-1.5 w-1.5 rounded-full bg-accent" />}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-[13px] text-vs-text">{title}</span>
        <span className="block text-[12px] text-vs-dim">{hint}</span>
        {preview.length > 0 && (
          <span className="mt-1 block truncate font-mono text-[11px] text-vs-dim">
            {preview.map((name) => `${prefix}${name}`).join('  ')}
          </span>
        )}
      </span>
    </button>
  );
}

function DialogButton({
  children,
  onClick,
  primary,
  danger,
  autoFocus,
}: {
  children: ReactNode;
  onClick: () => void;
  primary?: boolean;
  danger?: boolean;
  autoFocus?: boolean;
}) {
  return (
    <button
      type="button"
      autoFocus={autoFocus}
      onClick={onClick}
      className={`rounded-[2px] px-3 py-1 text-[13px] ${
        danger
          ? 'bg-red-700 text-white hover:bg-red-600'
          : primary
            ? 'bg-accent text-canvas hover:brightness-110'
            : 'border border-vs-border text-vs-text hover:bg-white/5'
      }`}
    >
      {children}
    </button>
  );
}
