import { useEffect, useMemo, useState } from 'react'
import { api, type ModuleName, type Release, type ReleaseChannel } from '../api'
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
  const [channel, setChannel] = useState<ReleaseChannel>('stable')

  const counts = useMemo(() => {
    const counts: Record<ReleaseChannel, number> = { stable: 0, beta: 0, nightly: 0 }
    for (const release of releases?.releases ?? []) counts[release.channel]++
    return counts
  }, [releases])

  // Start on stable, or on the first channel that has anything.
  useEffect(() => {
    if (!open || !releases) return
    if (counts.stable === 0) setChannel(CHANNELS.find((c) => counts[c.id] > 0)?.id ?? 'stable')
  }, [open, releases, counts])

  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => event.key === 'Escape' && setOpen(false)
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, setOpen])

  if (!open) return null
  const shown = (releases?.releases ?? []).filter((release) => release.channel === channel)
  const noSource = releases !== null && releases.source === null
  const unpublished = releases?.unpublished === true

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
        {!noSource && !unpublished && (
          <div className="dialog-body" style={{ paddingBottom: 12, overflow: 'visible' }}>
            <div className="segmented">
              {CHANNELS.map((c) => (
                <button key={c.id} className={channel === c.id ? 'on' : ''} onClick={() => setChannel(c.id)}>
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
          {releases?.stale && (
            <div className="banner">
              <Icon name="info" size={15} />
              <span>Couldn't check for new releases, so this is the list from last time. {releases.stale}</span>
            </div>
          )}
          {noSource && <NoSource onClose={() => setOpen(false)} />}
          {unpublished && <Unpublished />}
          {!releases && loading && <p className="hint" style={{ padding: 12 }}>Checking for releases…</p>}
          {releases && !noSource && !unpublished && shown.length === 0 && (
            <p className="hint" style={{ padding: 12 }}>
              Nothing in the {channel} channel yet.
            </p>
          )}
          {shown.map((release) => (
            <ReleaseRow key={release.version} release={release} />
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
          {releases?.source && (
            <span className="hint" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={releases.source}>
              Every download is verified
            </span>
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
        <span className="release-version">Voxear {release.version}</span>
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
          <button className="btn btn-sm" onClick={() => queueInstall(release.version, without)}>
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

function NoSource({ onClose }: { onClose: () => void }) {
  const setView = useStore((s) => s.setView)
  return (
    <div className="empty" style={{ marginTop: 32 }}>
      <h2>No release source yet</h2>
      <p>
        Voxear releases aren't published yet. Point the launcher at a release folder or URL in Settings, or install an
        engine archive from a file.
      </p>
      <div className="empty-actions">
        <button
          className="btn btn-primary"
          onClick={() => {
            onClose()
            setView('settings')
          }}
        >
          Open Settings
        </button>
      </div>
    </div>
  )
}
