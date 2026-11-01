import { useEffect, useState } from 'react'
import { api, failure, type Install } from '../api'
import { ChannelBadge, phaseText, ProgressBar } from '../components/Bits'
import { Icon } from '../components/Icon'
import { Rich } from '../components/Rich'
import { ago, bytes, pretty } from '../lib/format'
import { menuAt, useStore, type Job, type MenuEntry } from '../store'
import { reveal } from './projectActions'

export function InstallsView() {
  const installs = useStore((s) => s.installs)
  const jobs = useStore((s) => s.jobs)
  const loaded = useStore((s) => s.loaded)
  const home = useStore((s) => s.info?.userHome)
  const setInstallSheet = useStore((s) => s.setInstallSheet)
  const sizes = useSizes(installs)

  return (
    <section className="view">
      <div className="view-header">
        <h1 className="view-title">
          Installs {installs.length > 0 && <small>{installs.length}</small>}
        </h1>
        <button className="btn" onClick={(event) => menuAt(event, moreInstallOptions())}>
          More
          <Icon name="chevronDown" size={13} strokeWidth={2} />
        </button>
        <button className="btn btn-primary" onClick={() => setInstallSheet(true)}>
          <Icon name="download" size={15} />
          Install a version
        </button>
      </div>
      <div className="view-scroll">
        {jobs.map((job) => (
          <JobCard key={job.key} job={job} />
        ))}
        {loaded && installs.length === 0 && jobs.length === 0 && <NoInstalls />}
        <div className="list">
          {installs.map((install, index) => (
            <InstallRow key={install.path} install={install} index={index} size={sizes[install.path]} home={home} />
          ))}
        </div>
      </div>
    </section>
  )
}

export function moreInstallOptions(): MenuEntry[] {
  return [
    { label: 'Install from a file…', icon: 'file', run: () => void installFromFile() },
    { label: 'Locate an engine…', icon: 'folderOpen', run: () => void locateEngine() }
  ]
}

export async function installFromFile() {
  const path = await api.pickFile('Install a Voxear engine archive', 'Voxear engine archive', ['zst'])
  if (path) useStore.getState().queueFile(path)
}

export async function locateEngine() {
  const store = useStore.getState()
  const folder = await api.pickFolder('Locate a Voxear engine (a folder with engine.toml)')
  if (!folder) return
  try {
    const install = await api.locate(folder)
    await store.refresh()
    store.toast('ok', `Added Voxear ${install.version} from ${folder}`)
  } catch (error) {
    store.toast('error', failure(error).message)
  }
}

/** Disk use for each install, worked out in the background. */
function useSizes(installs: Install[]): Record<string, number> {
  const [sizes, setSizes] = useState<Record<string, number>>({})
  useEffect(() => {
    for (const install of installs) {
      if (install.missing || sizes[install.path] !== undefined) continue
      api.installSize(install.path).then(
        (size) => setSizes((current) => ({ ...current, [install.path]: size })),
        () => {}
      )
    }
  }, [installs, sizes])
  return sizes
}

function JobCard({ job }: { job: Job }) {
  const cancelJob = useStore((s) => s.cancelJob)
  const dismissJob = useStore((s) => s.dismissJob)
  const retryJob = useStore((s) => s.retryJob)
  return (
    <div className="job">
      <div className="engine-icon">
        <Icon name={job.state === 'failed' ? 'alert' : 'download'} size={16} style={{ color: job.state === 'failed' ? 'var(--danger)' : 'var(--accent-text)' }} />
      </div>
      <div className="job-text">
        <strong>{job.state === 'failed' ? `Couldn't install ${job.label}` : `Installing ${job.label}`}</strong>
        {job.state !== 'failed' && <span className="job-phase">{phaseText(job.progress, job.state === 'queued')}</span>}
      </div>
      <div className="row-actions compact" style={{ opacity: 1 }}>
        {job.state === 'failed' ? (
          <>
            <button className="btn btn-sm" onClick={() => retryJob(job.key)}>
              Try again
            </button>
            <button className="icon-btn" title="Dismiss" onClick={() => dismissJob(job.key)}>
              <Icon name="close" size={14} />
            </button>
          </>
        ) : (
          <button className="btn btn-sm btn-ghost" onClick={() => cancelJob(job.key)}>
            Cancel
          </button>
        )}
      </div>
      {job.state === 'failed' ? <div className="job-error">
          <Rich text={job.error ?? ''} />
        </div> : <ProgressBar progress={job.state === 'queued' ? null : job.progress} />}
    </div>
  )
}

function InstallRow({ install, index, size, home }: { install: Install; index: number; size: number | undefined; home: string | undefined }) {
  const store = useStore.getState
  const menu: MenuEntry[] = [
    { label: 'New project with this version', icon: 'plus', run: () => store().openNewProject(install.version) },
    { label: 'Show in files', icon: 'folder', run: () => void reveal(install.path) },
    ...moduleEntries(install),
    'divider',
    install.located
      ? { label: 'Forget', icon: 'close', danger: true, run: () => void uninstall(install) }
      : { label: 'Uninstall', icon: 'trash', danger: true, run: () => void uninstall(install) }
  ]
  return (
    <div className="row" style={{ animationDelay: `${Math.min(index, 12) * 18}ms` }} onContextMenu={(event) => menuAt(event, menu)}>
      <div className="engine-icon">
        <Icon name="cube" size={17} style={{ color: `var(--${install.channel})` }} />
      </div>
      <div className="row-main">
        <span className="row-name">Voxear {install.version}</span>
        <span className="row-path" title={install.path}>
          {pretty(install.path, home)}
        </span>
      </div>
      <div className="cell cell-channel">
        <ChannelBadge channel={install.channel} />
      </div>
      <div className="cell cell-tag">
        {install.missing ? (
          <span className="chip chip-warn">Folder not found</span>
        ) : install.located ? (
          <span className="tag">Located</span>
        ) : (
          install.modules.some((module) => module.name === 'manual' && module.installed) && (
            <span className="tag" title="The Voxear Manual is installed with this version">
              Manual
            </span>
          )
        )}
      </div>
      <span className="cell cell-size row-meta">{size !== undefined ? bytes(size) : ''}</span>
      <span className="cell cell-time row-meta" title={install.installed ? 'Installed' : undefined}>
        {install.installed ? ago(install.installed) : install.located ? 'Built elsewhere' : ''}
      </span>
      <div className="row-actions compact" style={{ opacity: 1 }}>
        <button className="icon-btn" title="More" onClick={(event) => menuAt(event, menu)}>
          <Icon name="more" size={18} strokeWidth={2.4} />
        </button>
      </div>
    </div>
  )
}

/** Add or remove each optional module, such as the manual. A located engine's files are never changed. */
function moduleEntries(install: Install): MenuEntry[] {
  if (install.located || install.missing) return []
  const entries: MenuEntry[] = []
  for (const module of install.modules) {
    if (module.installed) {
      entries.push({ label: `Remove the ${module.title}`, icon: 'close', run: () => void removeModule(install, module.name, module.title) })
    } else if (module.available) {
      entries.push({
        label: `Add the ${module.title}`,
        icon: 'download',
        run: () => useStore.getState().queueModule(install.version, module.name, module.title)
      })
    }
  }
  return entries.length ? ['divider', ...entries] : []
}

async function removeModule(install: Install, name: string, title: string) {
  const store = useStore.getState()
  const ok = await store.confirm({
    title: `Remove the ${title}?`,
    message: `It will be deleted from Voxear ${install.version}, which keeps working without it. You can add it again any time.`,
    confirm: 'Remove',
    danger: true
  })
  if (!ok) return
  try {
    await api.removeModule(install.version, name)
    await store.refresh()
    store.toast('ok', `Removed the ${title} from Voxear ${install.version}`)
  } catch (error) {
    store.toast('error', failure(error).message)
  }
}

async function uninstall(install: Install) {
  const store = useStore.getState()
  try {
    let message: string
    if (install.located) {
      message = `The launcher will stop listing it. Its files in ${install.path} stay where they are.`
    } else {
      const using = await api.projectsUsing(install.version)
      message = using.length
        ? `${listNames(using)} ${using.length === 1 ? 'is' : 'are'} pinned to it and won't open until it's installed again.`
        : 'Its files will be deleted. You can install it again any time.'
    }
    const ok = await store.confirm({
      title: install.located ? `Forget Voxear ${install.version}?` : `Uninstall Voxear ${install.version}?`,
      message,
      confirm: install.located ? 'Forget' : 'Uninstall',
      danger: true
    })
    if (!ok) return
    await api.uninstall(install.version)
    await store.refresh()
    void store.loadReleases()
    store.toast('ok', install.located ? `Forgot Voxear ${install.version}` : `Uninstalled Voxear ${install.version}`)
  } catch (error) {
    store.toast('error', failure(error).message)
  }
}

function listNames(names: string[]): string {
  if (names.length <= 2) return names.join(' and ')
  return `${names.slice(0, 2).join(', ')} and ${names.length - 2} more`
}

function NoInstalls() {
  const setInstallSheet = useStore((s) => s.setInstallSheet)
  return (
    <div className="empty">
      <svg className="empty-art" width="120" height="76" viewBox="0 0 160 100" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round">
        <path d="M56 30 L76 41 L56 52 L36 41 Z M36 41 V61 L56 72 V52 M76 41 V61 L56 72" />
        <path d="M104 30 L124 41 L104 52 L84 41 Z M84 41 V61 L104 72 V52 M124 41 V61 L104 72" opacity="0.45" strokeDasharray="3 3" />
      </svg>
      <h2>No engine versions yet</h2>
      <p>Install a version to create and open projects. Versions install side by side, so each project keeps the one it was made with.</p>
      <div className="empty-actions">
        <button className="btn btn-primary" onClick={() => setInstallSheet(true)}>
          <Icon name="download" size={15} />
          Install a version
        </button>
        <button className="btn" onClick={() => void locateEngine()}>
          Locate an engine
        </button>
      </div>
    </div>
  )
}
