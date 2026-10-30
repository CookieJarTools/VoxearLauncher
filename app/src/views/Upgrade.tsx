import { useEffect, useMemo, useState } from 'react'
import { api, failure, type Backup, type Project, type ReleaseNote, type UpgradeChoices, type UpgradeReport } from '../api'
import { ChannelBadge, ProgressBar, Switch } from '../components/Bits'
import { Icon } from '../components/Icon'
import { Rich } from '../components/Rich'
import { ago, bytes, pretty } from '../lib/format'
import { useStore } from '../store'
import { openProject, reveal } from './projectActions'

type Stage =
  | { kind: 'choose' }
  | { kind: 'running'; backup: { done: number; total: number } | null; steps: string[] }
  | { kind: 'done'; report: UpgradeReport }
  | { kind: 'failed'; message: string }

const RELATION: Record<string, string> = { upgrade: 'Newer', downgrade: 'Older', current: 'Current', pin: '' }

/** Moves a project to another engine version, or pins one that has none. The project is backed up first. */
export function UpgradeDialog() {
  const project = useStore((s) => s.upgrading)
  if (!project) return null
  return <Upgrade key={project.path} project={project} />
}

function Upgrade({ project }: { project: Project }) {
  const close = () => useStore.getState().openUpgrade(null)
  const home = useStore((s) => s.info?.userHome)
  const [choices, setChoices] = useState<UpgradeChoices | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [downgradeOk, setDowngradeOk] = useState(false)
  const [notes, setNotes] = useState<ReleaseNote[]>([])
  const [stage, setStage] = useState<Stage>({ kind: 'choose' })

  useEffect(() => {
    api.upgradeChoices(project.path).then(
      (choices) => {
        setChoices(choices)
        // Prefer moving forward, and the newest stable version that way; the list is newest first.
        const usable = choices.choices.filter((c) => c.direction !== 'current' && !c.missing)
        const forward = usable.filter((c) => c.direction === 'upgrade' || c.direction === 'pin')
        const pool = forward.length ? forward : usable
        setSelected((pool.find((c) => c.channel === 'stable') ?? pool[0])?.version ?? null)
      },
      (error) => setStage({ kind: 'failed', message: failure(error).message })
    )
  }, [project.path])

  const choice = useMemo(() => choices?.choices.find((c) => c.version === selected), [choices, selected])

  useEffect(() => {
    setDowngradeOk(false)
    setNotes([])
    if (!choice || choice.direction === 'downgrade') return
    let current = true
    api.releaseNotes(choices?.current ?? null, choice.version).then(
      (notes) => current && setNotes(notes),
      () => {}
    )
    return () => {
      current = false
    }
  }, [choice, choices?.current])

  const running = stage.kind === 'running'
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => event.key === 'Escape' && !running && close()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  const start = async () => {
    if (!choice) return
    setStage({ kind: 'running', backup: null, steps: [] })
    try {
      const report = await api.upgradeProject(project.path, choice.version, choice.direction === 'downgrade', (event) =>
        setStage((stage) => {
          if (stage.kind !== 'running') return stage
          if (event.kind === 'backingUp') return { ...stage, backup: { done: event.done, total: event.total } }
          if (event.kind === 'step') return { ...stage, steps: [...stage.steps, event.message] }
          return stage
        })
      )
      setStage({ kind: 'done', report })
      void useStore.getState().refresh()
    } catch (error) {
      setStage({ kind: 'failed', message: failure(error).message })
      void useStore.getState().refresh()
    }
  }

  const name = choices?.name ?? project.name
  const pinning = !choices?.current
  const others = choices?.choices.filter((c) => c.direction !== 'current') ?? []
  const verb = choice?.direction === 'downgrade' ? 'Downgrade' : choice?.direction === 'pin' ? 'Pin' : 'Upgrade'

  return (
    <div className="overlay" onMouseDown={() => !running && close()}>
      <div className="dialog upgrade" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="Change engine version">
        <div className="dialog-head">
          <div className="dialog-title">{pinning ? `Pin ${name} to an engine version` : `Change ${name}'s engine version`}</div>
          <button className="icon-btn" title="Close" onClick={close} disabled={running}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="dialog-body" style={{ display: 'grid', gap: 12 }}>
          {stage.kind === 'choose' && choices && (
            <>
              <p className="hint">
                {pinning ? `${name} isn't pinned to an engine version yet.` : `${name} is on Voxear ${choices.current}.`} The project is
                backed up first, so you can always go back.
              </p>
              {others.length === 0 ? (
                <div className="empty" style={{ margin: '12px auto' }}>
                  <p>No other engine version is installed.</p>
                  <div className="empty-actions">
                    <button
                      className="btn btn-primary"
                      onClick={() => {
                        close()
                        useStore.getState().setInstallSheet(true)
                      }}
                    >
                      <Icon name="download" size={15} />
                      Install a version
                    </button>
                  </div>
                </div>
              ) : (
                <div style={{ display: 'grid', gap: 2 }}>
                  {choices.choices.map((c) => (
                    <button
                      key={c.version}
                      className={`choice ${selected === c.version ? 'on' : ''}`}
                      disabled={c.direction === 'current' || c.missing}
                      onClick={() => setSelected(c.version)}
                    >
                      <span className="radio" />
                      <span className="release-version" style={{ flex: 1 }}>
                        Voxear {c.version}
                      </span>
                      <ChannelBadge channel={c.channel} />
                      {c.missing ? <span className="chip chip-warn">Folder not found</span> : RELATION[c.direction] && <span className="tag">{RELATION[c.direction]}</span>}
                    </button>
                  ))}
                </div>
              )}
              {choice?.direction === 'downgrade' && (
                <>
                  <div className="banner">
                    <Icon name="alert" size={15} />
                    <span>
                      Voxear {choice.version} is older than {choices.current}. Projects don't always open in older versions; the backup is
                      there if it doesn't.
                    </span>
                  </div>
                  <button className="switch-row" onClick={() => setDowngradeOk(!downgradeOk)}>
                    <span className="text">Downgrade anyway</span>
                    <Switch on={downgradeOk} />
                  </button>
                </>
              )}
              {notes.length > 0 && (
                <div className="field">
                  <span className="label">What's new</span>
                  <div className="notes">
                    {notes.map((note) => (
                      <a key={note.version} className="link" onClick={() => void api.openUrl(note.notes)}>
                        Voxear {note.version}
                      </a>
                    ))}
                  </div>
                </div>
              )}
            </>
          )}

          {stage.kind === 'running' && (
            <div className="steps" aria-live="polite">
              <div className={`step ${stage.steps.length === 0 ? 'current' : ''}`}>
                <Icon name={stage.steps.length === 0 ? 'spinner' : 'check'} size={14} strokeWidth={2} className={stage.steps.length === 0 ? 'spin' : undefined} />
                Backing up {name}
                {stage.backup && stage.backup.total > 0 && (
                  <span className="hint">
                    {bytes(stage.backup.done)} of {bytes(stage.backup.total)}
                  </span>
                )}
              </div>
              {stage.steps.length === 0 && <ProgressBar progress={stage.backup ? { phase: 'unpacking', done: stage.backup.done, total: stage.backup.total } : null} />}
              {stage.steps.map((step, index) => (
                <div key={index} className={`step ${index === stage.steps.length - 1 ? 'current' : ''}`}>
                  <Icon name={index === stage.steps.length - 1 ? 'spinner' : 'check'} size={14} strokeWidth={2} className={index === stage.steps.length - 1 ? 'spin' : undefined} />
                  {step}
                </div>
              ))}
            </div>
          )}

          {stage.kind === 'done' && (
            <div className="summary">
              <div className="summary-title">
                <Icon name="check" size={18} strokeWidth={2.2} />
                {name} is on Voxear {stage.report.to}
              </div>
              <ul className="changes">
                {stage.report.changes.map((change, index) => (
                  <li key={index}>
                    <Icon name="check" size={13} strokeWidth={2} />
                    <Rich text={change} />
                  </li>
                ))}
              </ul>
              {!stage.report.migrated && (
                <p className="hint">
                  Voxear {stage.report.to} can't convert projects yet, so only the pinned version changed. Nothing else in the project was
                  touched.
                </p>
              )}
              <div className="backup-line">
                <Icon name="archive" size={15} />
                <span title={stage.report.backup.archive}>
                  Backup from before · {bytes(stage.report.backup.size)} · {pretty(stage.report.backup.archive, home)}
                </span>
                <button className="btn btn-sm btn-ghost" onClick={() => void reveal(stage.report.backup.archive)}>
                  Show
                </button>
              </div>
            </div>
          )}

          {stage.kind === 'failed' && (
            <div className="banner banner-error">
              <Icon name="alert" size={15} />
              <span style={{ userSelect: 'text' }}>
                <Rich text={stage.message} />
              </span>
            </div>
          )}
        </div>

        <div className="dialog-foot">
          {stage.kind === 'failed' && (
            <button
              className="btn btn-ghost btn-sm"
              onClick={() => {
                close()
                useStore.getState().openBackups(project)
              }}
            >
              <Icon name="archive" size={14} />
              Backups
            </button>
          )}
          <div className="spacer" />
          {stage.kind === 'done' ? (
            <>
              <button className="btn btn-ghost" onClick={close}>
                Done
              </button>
              <button
                className="btn btn-primary"
                onClick={() => {
                  close()
                  const fresh = useStore.getState().projects.find((p) => p.path === project.path)
                  if (fresh) void openProject(fresh)
                }}
              >
                Open {name}
              </button>
            </>
          ) : (
            <>
              <button className="btn btn-ghost" onClick={close} disabled={running}>
                {stage.kind === 'failed' ? 'Close' : 'Cancel'}
              </button>
              {stage.kind !== 'failed' && (
                <button
                  className={`btn ${choice?.direction === 'downgrade' ? 'btn-danger' : 'btn-primary'}`}
                  onClick={() => void start()}
                  disabled={!choice || running || (choice.direction === 'downgrade' && !downgradeOk)}
                >
                  {running ? 'Working…' : choice ? `${verb} to ${choice.version}` : verb}
                </button>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  )
}

/** A project's backups: restore one into a new folder, show it, or delete it. */
export function BackupsDialog() {
  const project = useStore((s) => s.backupsOf)
  if (!project) return null
  return <Backups key={project.path} project={project} />
}

function Backups({ project }: { project: Project }) {
  const close = () => useStore.getState().openBackups(null)
  const home = useStore((s) => s.info?.userHome)
  const [backups, setBackups] = useState<Backup[] | null>(null)
  const [busy, setBusy] = useState<string | null>(null)

  const load = () =>
    api.listBackups(project.path).then(setBackups, (error) => useStore.getState().toast('error', failure(error).message))

  useEffect(() => {
    void load()
    const onKey = (event: KeyboardEvent) => event.key === 'Escape' && close()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // Loads once for the project this dialog was opened for.
  }, [project.path])

  const restore = async (backup: Backup) => {
    const store = useStore.getState()
    setBusy(backup.archive)
    try {
      const path = await api.restoreBackup(backup.archive)
      await store.refresh()
      close()
      store.setView('projects')
      store.setHighlight(path)
      store.toast('ok', `Restored ${backup.project} to ${pretty(path, home)}. The project it came from wasn't touched.`, {
        label: 'Show',
        run: () => void reveal(path)
      })
    } catch (error) {
      store.toast('error', failure(error).message)
    } finally {
      setBusy(null)
    }
  }

  const remove = async (backup: Backup) => {
    const store = useStore.getState()
    const ok = await store.confirm({
      title: 'Delete this backup?',
      message: `The backup of ${backup.project} from ${ago(backup.created)} (${bytes(backup.size)}) will be deleted. The project isn't affected.`,
      confirm: 'Delete',
      danger: true
    })
    if (!ok) return
    try {
      await api.deleteBackup(backup.archive)
      await load()
    } catch (error) {
      store.toast('error', failure(error).message)
    }
  }

  return (
    <div className="overlay" onMouseDown={close}>
      <div className="dialog backups" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="Backups">
        <div className="dialog-head">
          <div className="dialog-title">Backups of {project.name}</div>
          <button className="icon-btn" title="Close" onClick={close}>
            <Icon name="close" size={15} />
          </button>
        </div>
        <div className="dialog-body" style={{ display: 'grid', gap: 2 }}>
          {backups?.length === 0 && (
            <div className="empty" style={{ margin: '24px auto' }}>
              <h2>No backups yet</h2>
              <p>One is made every time {project.name} changes engine version.</p>
            </div>
          )}
          {backups?.map((backup, index) => (
            <div key={backup.archive} className="backup" style={{ animationDelay: `${index * 18}ms` }}>
              <div style={{ display: 'grid' }}>
                <span className="release-version" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  {backup.from ? `Voxear ${backup.from}` : 'Not pinned'}
                  <Icon name="arrowRight" size={13} strokeWidth={2} style={{ color: 'var(--text-faint)' }} />
                  {backup.to}
                </span>
                <span className="release-meta">
                  {ago(backup.created)} · {bytes(backup.size)} · {backup.files} files
                </span>
              </div>
              <div style={{ display: 'flex', gap: 4 }}>
                <button className="btn btn-sm" onClick={() => void restore(backup)} disabled={busy !== null} title="Unpacks it into a new folder beside the project">
                  {busy === backup.archive ? 'Restoring…' : 'Restore a copy'}
                </button>
                <button className="icon-btn" title="Show in files" onClick={() => void reveal(backup.archive)}>
                  <Icon name="folder" size={15} />
                </button>
                <button className="icon-btn" title="Delete" onClick={() => void remove(backup)}>
                  <Icon name="trash" size={15} />
                </button>
              </div>
              {backup.changes.length > 0 && (
                <ul className="changes">
                  {backup.changes.map((change, i) => (
                    <li key={i}>
                      <Icon name="check" size={12} strokeWidth={2} />
                      <Rich text={change} />
                    </li>
                  ))}
                </ul>
              )}
            </div>
          ))}
        </div>
        <div className="dialog-foot">
          <span className="hint">Restoring unpacks a copy beside the project and adds it to the list. Nothing is overwritten.</span>
        </div>
      </div>
    </div>
  )
}
