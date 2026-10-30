import { useEffect, useState } from 'react'
import { api, failure, type Settings } from '../api'
import { ProgressBar } from '../components/Bits'
import { Icon } from '../components/Icon'
import { ago, bytes, pretty } from '../lib/format'
import { useStore } from '../store'
import { reveal } from './projectActions'

export function SettingsView() {
  const info = useStore((s) => s.info)
  const toast = useStore((s) => s.toast)
  const [saved, setSaved] = useState<Settings | null>(null)
  const [releases, setReleases] = useState('')
  const [keys, setKeys] = useState('')
  const [location, setLocation] = useState('')
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    api.getSettings().then(
      (settings) => {
        setSaved(settings)
        setReleases(settings.releases ?? '')
        setKeys((settings.trusted_keys ?? []).join('\n'))
        setLocation(settings.project_location ?? '')
      },
      (error) => toast('error', failure(error).message)
    )
  }, [toast])

  const next: Settings = {
    releases: releases.trim() || undefined,
    trusted_keys: keys
      .split('\n')
      .map((key) => key.trim())
      .filter(Boolean),
    project_location: location.trim() || undefined
  }
  const dirty =
    saved !== null &&
    ((saved.releases ?? '') !== (next.releases ?? '') ||
      (saved.trusted_keys ?? []).join('\n') !== (next.trusted_keys ?? []).join('\n') ||
      (saved.project_location ?? '') !== (next.project_location ?? ''))

  const save = async () => {
    setSaving(true)
    try {
      // Keeps what this form doesn't edit, such as the launcher channel.
      const merged = { ...(await api.getSettings()), ...next }
      await api.saveSettings(merged)
      setSaved(merged)
      toast('ok', 'Settings saved')
      const store = useStore.getState()
      void store.loadReleases()
      void api.appInfo().then((info) => useStore.setState({ info }))
    } catch (error) {
      toast('error', failure(error).message)
    } finally {
      setSaving(false)
    }
  }

  const browseReleases = async () => {
    const file = await api.pickFile('Choose a releases.toml', 'Release manifest', ['toml'])
    if (file) setReleases(file)
  }

  const browseLocation = async () => {
    const folder = await api.pickFolder('Where should new projects go?', location || info?.defaultLocation)
    if (folder) setLocation(folder)
  }

  return (
    <section className="view">
      <div className="view-header">
        <h1 className="view-title">Settings</h1>
        <button className="btn btn-primary" onClick={() => void save()} disabled={!dirty || saving}>
          {saving ? 'Saving…' : 'Save'}
        </button>
      </div>
      <div className="view-scroll">
        <div className="settings">
          <div className="settings-section">
            <h2>Engine releases</h2>
            <div className="field">
              <span className="label">Release source</span>
              <div className="input-row">
                <input
                  className="input"
                  value={releases}
                  placeholder="https://…/releases.toml, or a releases.toml on disk"
                  onChange={(e) => setReleases(e.target.value)}
                  spellCheck={false}
                />
                <button className="btn" onClick={() => void browseReleases()}>
                  Browse
                </button>
              </div>
              <span className="hint">
                Where engine versions are listed. Official releases aren't published yet, so for now this points at a release
                folder. VOXUP_RELEASES overrides it.
              </span>
            </div>
            <label className="field">
              <span className="label">Extra trusted keys</span>
              <textarea
                className="input"
                value={keys}
                placeholder="One minisign public key per line"
                onChange={(e) => setKeys(e.target.value)}
                spellCheck={false}
              />
              <span className="hint">
                Every manifest and download must be signed by a trusted key. Add keys here for engine builds you host
                yourself.
              </span>
            </label>
          </div>

          <div className="settings-section">
            <h2>Projects</h2>
            <div className="field">
              <span className="label">New projects go in</span>
              <div className="input-row">
                <input
                  className="input"
                  value={location}
                  placeholder={info?.defaultLocation}
                  onChange={(e) => setLocation(e.target.value)}
                  spellCheck={false}
                />
                <button className="btn" onClick={() => void browseLocation()}>
                  Browse
                </button>
              </div>
              <span className="hint">The last folder you created a project in is remembered here.</span>
            </div>
          </div>

          <LauncherUpdates />

          <div className="settings-section">
            <h2>About</h2>
            <dl className="facts">
              <dt>Launcher</dt>
              <dd>{info?.version}</dd>
              <span />
              <dt>Platform</dt>
              <dd>{info?.platform}</dd>
              <span />
              <dt>Launcher folder</dt>
              <dd title={info?.home}>{info ? pretty(info.home, info.userHome) : ''}</dd>
              <button className="btn btn-sm btn-ghost" onClick={() => info && void reveal(info.home)}>
                <Icon name="folder" size={14} />
                Show
              </button>
              <dt>Settings file</dt>
              <dd title={info?.settingsFile}>{info ? pretty(info.settingsFile, info.userHome) : ''}</dd>
              <span />
            </dl>
            <span className="hint">Everything here can also be done from a terminal with voxup.</span>
          </div>
        </div>
      </div>
    </section>
  )
}

/** The launcher's own updates: its channel, a check, and what the last check found. */
function LauncherUpdates() {
  const update = useStore((s) => s.launcherUpdate)
  const check = useStore((s) => s.checkLauncherUpdate)
  const install = useStore((s) => s.installLauncherUpdate)
  const toast = useStore((s) => s.toast)
  const [channel, setChannel] = useState<'stable' | 'beta'>('stable')

  useEffect(() => {
    api.getSettings().then((settings) => setChannel(settings.launcher_channel ?? 'stable'), () => {})
  }, [])

  // The channel is a preference, so it saves at once, and checks again.
  const choose = async (next: 'stable' | 'beta') => {
    setChannel(next)
    try {
      const settings = await api.getSettings()
      await api.saveSettings({ ...settings, launcher_channel: next })
      void check()
    } catch (error) {
      toast('error', failure(error).message)
    }
  }

  let status: string
  if (update.state === 'checking') status = 'Checking…'
  else if (update.state === 'downloading') status = `Downloading ${update.version}${update.total ? ` · ${bytes(update.done)} of ${bytes(update.total)}` : ''}`
  else if (update.state === 'ready') status = `${update.version} is installed and starts when you restart the launcher.`
  else if (update.error) status = update.error
  else if (!update.status) status = 'Not checked yet.'
  else if (update.status.kind === 'notSetUp') status = update.status.reason
  else if (update.status.kind === 'nothingPublished') status = 'No launcher updates are published yet.'
  else if (update.status.kind === 'upToDate') status = `${update.status.version} is the newest ${channel} version.`
  else status = `${update.status.version} is out.`
  const checked = update.state === 'idle' && update.checkedAt ? ` Checked ${ago(new Date(update.checkedAt).toISOString())}.` : ''

  return (
    <div className="settings-section">
      <h2>Launcher updates</h2>
      <div className="field">
        <span className="label">Channel</span>
        <div className="segmented" style={{ justifySelf: 'start' }}>
          {(['stable', 'beta'] as const).map((id) => (
            <button key={id} className={channel === id ? 'on' : ''} onClick={() => void choose(id)}>
              {id === 'stable' ? 'Stable' : 'Beta'}
            </button>
          ))}
        </div>
        <span className="hint">Engine versions have their own channels; this is only for the launcher.</span>
      </div>
      <div className="input-row" style={{ alignItems: 'center' }}>
        <span className={update.state === 'idle' && update.error ? 'error-text' : 'hint'} style={{ flex: 1 }}>
          {status}
          {checked}
        </span>
        {update.state === 'idle' && update.status?.kind === 'available' ? (
          <button className="btn btn-primary btn-sm" onClick={() => void install()}>
            Update to {update.status.version}
          </button>
        ) : (
          <button className="btn btn-sm" onClick={() => void check()} disabled={update.state !== 'idle'}>
            Check now
          </button>
        )}
      </div>
      {update.state === 'downloading' && (
        <ProgressBar progress={update.total ? { phase: 'downloading', done: update.done, total: update.total } : null} />
      )}
    </div>
  )
}
