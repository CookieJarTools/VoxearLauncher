import { useEffect, useMemo, useState } from 'react'
import { api, failure, type ModuleName, type Release, type ReleaseChannel, type ReleaseSource } from '../api'
import { Icon } from '../components/Icon'
import { Rich } from '../components/Rich'
import { ago, bytes } from '../lib/format'
import { useStore } from '../store'
import { installFromFile, locateEngine } from './InstallsView'

const CHANNELS: { id: ReleaseChannel; label: string }[] = [
  { id: 'stable', label: 'Stable' },
  { id: 'beta', label: 'Beta' },
  { id: 'nightly', label: 'Nightly' }
]

/** Choose an engine release to install. */
export function InstallSheet() {
  const open = useStore((s) => s.installSheet)
  const setOpen = useStore((s) => s.setInstallSheet)
  const releases = useStore((s) => s.releases)
  const error = useStore((s) => s.releasesError)
  const loading = useStore((s) => s.releasesLoading)
  const loadReleases = useStore((s) => s.loadReleases)
  const toast = useStore((s) => s.toast)
  const [channel, setChannel] = useState<ReleaseChannel>('stable')

  const counts = useMemo(() => {
    const counts: Record<ReleaseChannel, number> = { stable: 0, beta: 0, nightly: 0 }
    for (const release of releases?.releases ?? []) counts[release.channel]++
    return counts
  }, [releases])
  // Nightlies show when the setting says, or once one is installed.
  const channels = useMemo(() => CHANNELS.filter((c) => c.id !== 'nightly' || releases?.nightlies), [releases])

  // Start on stable, or on the first channel that has anything.
  useEffect(() => {
    if (!open || !releases) return
    if (counts.stable === 0) setChannel(channels.find((c) => counts[c.id] > 0)?.id ?? 'stable')
  }, [open, releases, counts, channels])

  const showNightlies = async () => {
    try {
      await api.saveSettings({ ...(await api.getSettings()), nightlies: true })
      setChannel('nightly')
      void loadReleases()
    } catch (error) {
      toast('error', failure(error).message)
    }
  }

  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => event.key === 'Escape' && setOpen(false)
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, setOpen])

  if (!open) return null
  // A channel the list no longer shows, such as nightlies just turned off, falls back to stable.
  const current = channels.some((c) => c.id === channel) ? channel : 'stable'
  const shown = (releases?.releases ?? []).filter((release) => release.channel === current)
  const sources = releases?.sources ?? []
  const unpublished = sources.length > 0 && sources.every((source) => source.state === 'unpublished')

  return (
    <div className="overlay" onMouseDown={() => setOpen(false)}>
      <div className="dialog install-sheet" onMouseDown={(e) => e.stopPropagation()} role="dialog" aria-label="Install a version">
        <div className="dialog-head">
          <div className="dialog-title">Install a version</div>
          <button className="icon-btn" title="Check again" onClick={() => void loadReleases()} disabled={loading}>
            <Icon name="refresh" size={15} className={loading ? 'spin' : undefined} />
          </button>
          <button className="icon-btn" title="Close" onClick={() => setOpen(false)}>
            <Icon name="close" size={15} />
          </button>
        </div>
        {!unpublished && (
          <div className="dialog-body" style={{ paddingBottom: 12, overflow: 'visible' }}>
            <div className="segmented">
              {channels.map((c) => (
                <button key={c.id} className={current === c.id ? 'on' : ''} onClick={() => setChannel(c.id)}>
                  {c.label}
                  <span className="count">{counts[c.id] || ''}</span>
                </button>
              ))}
            </div>
          </div>
        )}
        <div className="dialog-body" style={{ flex: 1, display: 'grid', alignContent: 'start', gap: 6 }}>
          {error && (
            <div className="banner banner-error">
              <Icon name="alert" size={15} />
              <span style={{ flex: 1 }}>
                <Rich text={error} />
              </span>
              <button className="btn btn-sm" onClick={() => void loadReleases()}>
                Try again
              </button>
            </div>
          )}
          {sources.map((source) => (
            <SourceProblem key={source.name} source={source} />
          ))}
          {unpublished && <Unpublished />}
          {!releases && loading && <p className="hint" style={{ padding: 12 }}>Checking for releases…</p>}
          {releases && !unpublished && shown.length === 0 && (
            <p className="hint" style={{ padding: 12 }}>
              Nothing in the {current} channel yet.
            </p>
          )}
          {shown.map((release) => (
            <ReleaseRow key={`${release.source ?? ''}/${release.version}`} release={release} />
          ))}
        </div>
        <div className="dialog-foot">
          <button className="btn btn-ghost btn-sm" onClick={() => void installFromFile()}>
            <Icon name="file" size={14} />
            From a file…
          </button>
          <button className="btn btn-ghost btn-sm" onClick={() => void locateEngine()}>
            <Icon name="folderOpen" size={14} />
            Locate an engine…
          </button>
          <div className="spacer" />
          {releases && !releases.nightlies && counts.nightly > 0 ? (
            <button className="btn btn-ghost btn-sm" onClick={() => void showNightlies()}>
              Show nightlies
            </button>
          ) : (
            releases && <span className="hint">Every download is verified</span>
          )}
        </div>
      </div>
    </div>
  )
}

function ReleaseRow({ release }: { release: Release }) {
  const job = useStore((s) => s.jobs.find((j) => j.key === release.version && j.state !== 'failed'))
  const queueInstall = useStore((s) => s.queueInstall)
  // Optional parts, such as the manual and the Dedicated Server, come along unless they're unticked.
  const [without, setWithout] = useState<ModuleName[]>([])
  const offered = !release.installed && !release.needsNewerLauncher && release.modules.length > 0
  return (
    <div className="release">
      <div className="grow">
        <span className="release-version">
          Voxear {release.version}
          {release.source && (
            <span className="tag" style={{ marginLeft: 8 }} title={`From ${release.source}`}>
              {release.source}
            </span>
          )}
        </span>
        <span className="release-meta">
          {ago(release.published)} · {bytes(release.size)}
          {release.notes && (
            <>
              {' · '}
              <a className="link" onClick={() => void api.openUrl(release.notes!)}>
                Release notes
              </a>
            </>
          )}
        </span>
        {offered && (
          <span className="module-options">
            {release.modules.map((module) => (
              <label key={module.name} className="check">
                <input
                  type="checkbox"
                  checked={!without.includes(module.name)}
                  disabled={Boolean(job)}
                  onChange={(event) =>
                    setWithout((current) =>
                      event.target.checked ? current.filter((name) => name !== module.name) : [...current, module.name]
                    )
                  }
                />
                {module.title}
                <span className="release-meta">{bytes(module.size)}</span>
              </label>
            ))}
          </span>
        )}
      </div>
      <div className="cell cell-action">
        {release.installed ? (
          <span className="chip chip-faint">
            <Icon name="check" size={12} strokeWidth={2.2} />
            Installed
          </span>
        ) : release.needsNewerLauncher ? (
          <span className="chip chip-warn" title="This release needs a newer launcher; update the launcher first">
            Needs a newer launcher
          </span>
        ) : job ? (
          <span className="chip">{job.state === 'queued' ? 'Waiting' : 'Installing…'}</span>
        ) : (
          <button className="btn btn-sm" onClick={() => queueInstall(release.version, without, release.source ?? 'Official')}>
            <Icon name="download" size={14} />
            Install
          </button>
        )}
      </div>
    </div>
  )
}

function Unpublished() {
  return (
    <div className="empty" style={{ marginTop: 32 }}>
      <h2>No engine releases yet</h2>
      <p>
        Nothing has been published yet. Until the first release, install an engine archive from a file, or locate an engine
        you built yourself.
      </p>
      <div className="empty-actions">
        <button className="btn" onClick={() => void installFromFile()}>
          <Icon name="file" size={14} />
          Install from a file…
        </button>
        <button className="btn" onClick={() => void locateEngine()}>
          <Icon name="folderOpen" size={14} />
          Locate an engine…
        </button>
      </div>
    </div>
  )
}

/** A release list that couldn't be read, or only from the copy saved last time. */
function SourceProblem({ source }: { source: ReleaseSource }) {
  if (source.state !== 'failed' && source.state !== 'stale') return null
  const which = source.official ? 'the official releases' : source.name
  const failed = source.state === 'failed'
  return (
    <div className={failed ? 'banner banner-error' : 'banner'} title={source.url}>
      <Icon name={failed ? 'alert' : 'info'} size={15} />
      <span style={{ flex: 1 }}>
        {failed ? `Couldn't read ${which}. ` : `Couldn't check ${which}, so this is the list from last time. `}
        {source.error && <Rich text={source.error} />}
      </span>
    </div>
  )
}
